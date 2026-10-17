//! Dosya seçici (`nuxut-filechooser`): portal `FileChooser` isteği başına açılan kısa ömürlü
//! pencere. Tasarım: `docs/design/file-chooser.md`; sözleşme `nuxut_portal_backend::chooser::protocol`;
//! UI'sız katman `services::file_chooser`.
//!
//! - xdg_toplevel pencere, app-id `nuxut-filechooser` (Hyprland kuralı: yüzen, ortalı).
//! - Üst satır: geri, üst klasör, klasör adı (Ctrl+L konum alanı), görünüm, gizli dosyalar, yeni
//!   klasör, arama. Solda yerler (`List`, gruplu), ortada dosyalar (`List` seçim kipi ya da
//!   `FileGrid`, Ctrl+1/2), altta ad alanı (kaydetme), filtre, seçim kutuları, Vazgeç / kabul.
//! - Izgara küçük resimleri yalnız görünen hücreler için, yalnız yalıtılmış işçide
//!   (`services::file_chooser::thumbs`, ADR-0007).
//! - Dizin okuma, klasör oluşturma işçi thread'lerinde; ana thread'de dosya sistemi IO'su yok
//!   (var olma denetimi son okumadan).
//! - Sonuç [`ChooserSetup::result`]'a yazılır; `run` dönünce ikili stdout'a tek satır basar.

mod access;

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nuxut::i18n::t;
use nuxut::render::{ImageData, TextureOptions};
use nuxut::ui::{
    Button as UiButton, ButtonVariant, Chip, ChipEvent, ChipKind, Dialog, DialogAction, DialogActionKind, DialogEvent, EmptyState, FileGrid, FileGridEvent,
    FileItem, Icon, IconButton, IconButtonVariant, IconName, Label, List, ListDensity, ListEvent, ListItem, ListSelectEvent, ListTrailing, Mood, PathBar,
    PathBarEvent, PathSegment, SearchField, Select, SelectEvent, TextInput, TextInputEvent, TextRole, Ui, text_style,
};
use nuxut::wayland::PasteRequestId;
use nuxut::{
    App, Context, CursorShape, ImeEvent, Key, KeyEvent, KeyState, Modifiers, OutputId, OutputInfo, Painter, Point, PointerEvent,
    PointerKind, Rect, Size, SurfaceId, SurroundingText, TextInputState, TextPurpose, TimerId, WindowSpec,
};
use nuxut_portal_backend::chooser::protocol::{ChooserRequest, ChooserResult, Mode, encode_path};

use crate::services::file_chooser::listing::{Entry, Kind, Link, ListError, ListResult, Lister, Listing};
use crate::services::file_chooser::mime::MimeDb;
use crate::services::file_chooser::names::{NameError, check_name, resolve_location, save_files_targets, search_matches};
use crate::services::file_chooser::places::{self, UserDir};
use crate::services::file_chooser::thumbs::{Thumb, ThumbJob, Thumbnailer};
use crate::services::removable_media::{MediaEvent, MediaOptions, MediaService, Volume};

pub const APP_ID: &str = "nuxut-filechooser";
const WINDOW_SIZE: (u32, u32) = (960, 640);
const MIN_SIZE: (u32, u32) = (640, 420);

/// Başlatma parametreleri.
#[derive(Debug)]
pub struct ChooserSetup {
    pub request: ChooserRequest,
    /// Sonuç; `None` = vazgeçildi.
    pub result: Arc<Mutex<Option<ChooserResult>>>,
    /// stdin'i yaşam hattı olarak izle (portal kapatınca iptal). Testte kapalı.
    pub watch_stdin: bool,
}

#[derive(Debug)]
pub enum Message {
    Listed(ListResult),
    Media(MediaEvent),
    FolderCreated(Result<OsString, std::io::ErrorKind>),
    /// Izgara küçük resmi (yalıtılmış çözüm; `services::file_chooser::thumbs`).
    Thumb(Thumb),
    /// stdin kapandı: portal isteği kapattı ya da öldü.
    Lifeline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Places,
    Files,
    Search,
    Location,
    Name,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Modal {
    None,
    /// Üzerine yazma onayı (hedef yol).
    Overwrite(PathBuf),
    NewFolder,
}

#[derive(Debug, Clone)]
enum PlaceTarget {
    Path(PathBuf),
    /// Çıkarılabilir birim: bağlı değilse tıklama bağlar.
    Volume { id: String, mount: Option<PathBuf> },
}

#[derive(Debug, Clone)]
struct Place {
    label: String,
    icon: IconName,
    target: PlaceTarget,
}

/// Dosya görünümü (oturum boyunca; kalıcı tercih yok).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    List,
    Grid,
}

/// Küçük resim istenen dosya: addan resim türü (içerik koklanmaz).
fn wants_thumbnail(entry: &Entry, mime: &MimeDb) -> bool {
    entry.is_regular() && mime.guess(&entry.display).starts_with("image/")
}

/// Görünür aralığın iki yanına eklenen önden yükleme payı (hücre).
const THUMB_PREFETCH: usize = 12;

/// Dizin durumu.
#[derive(Debug)]
enum DirState {
    Loading,
    Ready(Listing),
    Failed(ListError),
}

/// Tema/dil değişince yeniden kurulan bileşenler.
struct Widgets {
    back: IconButton,
    up: IconButton,
    /// Liste ↔ ızgara.
    view: IconButton,
    hidden: IconButton,
    new_folder: IconButton,
    path: PathBar,
    search: SearchField,
    places: List,
    files: List,
    grid: FileGrid,
    empty: EmptyState,
    name_label: Label,
    name: TextInput,
    filter: Option<Select>,
    choice_labels: Vec<Label>,
    /// `request.choices` ile aynı sıra: onay kutusu chip, liste Select.
    choices: Vec<ChoiceWidget>,
    cancel: UiButton,
    accept: UiButton,
    status: Label,
    dialog: Dialog,
    dialog_body: Label,
    folder_name: TextInput,
}

enum ChoiceWidget {
    Toggle(Box<Chip>),
    List(Box<Select>),
}

pub struct FileChooser {
    setup: ChooserSetup,
    started: bool,
    window: Option<SurfaceId>,
    size: Size,
    w: Option<Widgets>,
    home: PathBuf,
    mime: MimeDb,
    lister: Lister,
    media: Option<MediaService>,
    volumes: Vec<Volume>,
    /// Bağlanması istenen birim (bağlanınca açılır).
    pending_volume: Option<String>,
    places: Vec<Place>,
    dir: PathBuf,
    state: DirState,
    back_stack: Vec<PathBuf>,
    forward_stack: Vec<PathBuf>,
    /// Konum alanından dosya yolu verildiyse: üst klasöre gidilip bu ad seçilir.
    pending_select: Option<OsString>,
    /// `files` satırlarının `Listing::entries` indisleri.
    visible: Vec<usize>,
    /// Seçili adlar: etkin görünümün (`List`/`FileGrid`) seçiminden türetilir.
    selected: Vec<OsString>,
    view_mode: ViewMode,
    scale: f32,
    /// Izgara küçük resimleri: ilk ızgara gösteriminde kurulur (işçi yoksa önizleme yok).
    thumbs: Option<Thumbnailer>,
    /// Izgara öğeleri her yeniden kurulduğunda artar (eski sonuç atılır).
    thumb_gen: u64,
    /// Bu nesilde istenmiş/bitmiş öğeler (yeniden istenmez).
    thumb_seen: std::collections::HashSet<usize>,
    /// Gelen, dokuya yüklenmeyi bekleyen küçük resimler (yükleme çizimde: painter gerekir).
    thumb_ready: Vec<(usize, nuxut_image::Image)>,
    /// Son istenen görünür aralık (değişmedikçe yeniden istenmez).
    thumb_range: std::ops::Range<usize>,
    show_hidden: bool,
    filter: Option<usize>,
    choice_values: Vec<String>,
    focus: Focus,
    location_open: bool,
    /// `PathBar` parçalarının yolları (aynı sıra).
    path_targets: Vec<PathBuf>,
    modal: Modal,
    status: Option<String>,
    mods: Modifiers,
    paste: Option<(PasteRequestId, Focus)>,
    wake: Option<TimerId>,
    parent_set: bool,
    done: bool,
    /// Ekran okuyucu: tıklanabilir düğüm → işaretçi noktası (son ağaçtan; `access.rs`).
    access_points: std::collections::HashMap<nuxut::access::NodeId, Point>,
    /// Ekran okuyucu: `Focus` eylemi alan metin alanları.
    access_focus: Vec<(nuxut::access::NodeId, Focus)>,
    /// Son AT tıklaması: bu andan [`access::ASSISTIVE_ACTIVATION_GUARD`] boyunca işaretçi çift tıkı
    /// (satırı açma/kabul) yok sayılır — çift tıkın bir yarısı AT olabilir.
    assistive_click: Option<Instant>,
}

impl std::fmt::Debug for FileChooser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileChooser")
            .field("mode", &self.setup.request.mode)
            .field("visible", &self.visible.len())
            .field("selected", &self.selected.len())
            .finish_non_exhaustive()
    }
}

fn icon_for(entry: &Entry, mime: &MimeDb) -> IconName {
    if entry.is_dir() {
        return IconName::Folder;
    }
    if entry.link == Link::Broken {
        return IconName::LinkOff;
    }
    let m = mime.guess(&entry.display);
    match m.split_once('/').map_or("", |(a, _)| a) {
        "image" => IconName::Image,
        "audio" => IconName::Music,
        "video" => IconName::Videocam,
        "text" => IconName::File,
        _ if m.contains("zip") || m.contains("tar") || m.contains("compressed") || m.contains("archive") => IconName::Package,
        _ => IconName::File,
    }
}

fn user_dir_label(d: UserDir) -> String {
    match d {
        UserDir::Desktop => t!("shell-filechooser-place-desktop"),
        UserDir::Documents => t!("shell-filechooser-place-documents"),
        UserDir::Download => t!("shell-filechooser-place-downloads"),
        UserDir::Pictures => t!("shell-filechooser-place-pictures"),
        UserDir::Music => t!("shell-filechooser-place-music"),
        UserDir::Videos => t!("shell-filechooser-place-videos"),
    }
}

fn user_dir_icon(d: UserDir) -> IconName {
    match d {
        UserDir::Desktop => IconName::Desktop,
        UserDir::Documents => IconName::File,
        UserDir::Download => IconName::Download,
        UserDir::Pictures => IconName::Image,
        UserDir::Music => IconName::Music,
        UserDir::Videos => IconName::Videocam,
    }
}

/// Satırın sağındaki metin: dosyada boyut · tarih, klasörde tarih.
fn entry_meta(e: &Entry) -> String {
    let date = e.modified.and_then(|m| {
        let dt: chrono::DateTime<chrono::Local> = m.into();
        use chrono::Datelike;
        crate::services::clock::time_locale().format_ymd(dt.year(), dt.month(), dt.day(), nuxut::locale::DateStyle::Short).ok()
    });
    let size = (e.kind == Kind::File && e.link != Link::Broken).then(|| crate::services::removable_media::size_text(e.size));
    match (size, date) {
        (Some(s), Some(d)) => format!("{s} · {d}"),
        (Some(s), None) => s,
        (None, Some(d)) => d,
        (None, None) => String::new(),
    }
}

impl FileChooser {
    pub fn new(setup: ChooserSetup) -> Self {
        let home = std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
        let req = &setup.request;
        let filter = req.current_filter.or((!req.filters.is_empty()).then_some(0));
        let choice_values = req.choices.iter().map(|c| c.initial_value()).collect();
        let dir = req
            .current_folder
            .as_deref()
            .and_then(nuxut_portal_backend::chooser::protocol::decode_path)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.clone());
        let focus = if req.mode == Mode::Save { Focus::Name } else { Focus::Files };
        Self {
            setup,
            started: false,
            window: None,
            size: Size::ZERO,
            w: None,
            home,
            mime: MimeDb::default(),
            lister: Lister::new(),
            media: None,
            volumes: Vec::new(),
            pending_volume: None,
            places: Vec::new(),
            dir,
            state: DirState::Loading,
            back_stack: Vec::new(),
            forward_stack: Vec::new(),
            pending_select: None,
            visible: Vec::new(),
            selected: Vec::new(),
            view_mode: ViewMode::List,
            scale: 1.0,
            thumbs: None,
            thumb_gen: 0,
            thumb_seen: Default::default(),
            thumb_ready: Vec::new(),
            thumb_range: 0..0,
            show_hidden: false,
            filter,
            choice_values,
            focus,
            location_open: false,
            path_targets: Vec::new(),
            modal: Modal::None,
            status: None,
            mods: Modifiers::default(),
            paste: None,
            wake: None,
            parent_set: false,
            done: false,
            access_points: Default::default(),
            assistive_click: None,
            access_focus: Vec::new(),
        }
    }

    fn req(&self) -> &ChooserRequest {
        &self.setup.request
    }

    fn title(&self) -> String {
        let r = self.req();
        if !r.title.is_empty() {
            return r.title.clone();
        }
        match (r.mode, r.directory) {
            (Mode::Open, true) => t!("shell-filechooser-title-folder"),
            (Mode::Open, false) => t!("shell-filechooser-title-open"),
            _ => t!("shell-filechooser-title-save"),
        }
    }

    fn accept_label(&self) -> String {
        let r = self.req();
        if let Some(l) = &r.accept_label {
            return l.clone();
        }
        match (r.mode, r.directory) {
            (Mode::Open, true) => t!("shell-filechooser-select"),
            (Mode::Open, false) => t!("shell-filechooser-open"),
            _ => t!("shell-filechooser-save"),
        }
    }

    fn redraw(&self, cx: &mut Context<Self>) {
        if let Some(s) = self.window {
            cx.request_frame(s);
        }
    }

    // --- Sonuç ---------------------------------------------------------------------------

    fn finish(&mut self, cx: &mut Context<Self>, paths: Vec<PathBuf>) {
        if self.done {
            return;
        }
        self.done = true;
        let choices = self.req().choices.iter().zip(&self.choice_values).map(|(c, v)| (c.id.clone(), v.clone())).collect();
        let filter = if self.req().mode == Mode::SaveFiles { None } else { self.filter };
        let result = ChooserResult { paths: paths.iter().map(|p| encode_path(p)).collect(), choices, filter };
        tracing::info!(?result, "seçim yapıldı");
        if let Ok(mut r) = self.setup.result.lock() {
            *r = Some(result);
        }
        cx.quit();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.done {
            return;
        }
        self.done = true;
        tracing::info!("seçici kapatıldı (vazgeçildi)");
        cx.quit();
    }

    // --- Yerler --------------------------------------------------------------------------

    fn build_places(&mut self) {
        let mut out = vec![Place { label: t!("shell-filechooser-place-home"), icon: IconName::Home, target: PlaceTarget::Path(self.home.clone()) }];
        for (d, p) in places::user_dirs(&self.home) {
            out.push(Place { label: user_dir_label(d), icon: user_dir_icon(d), target: PlaceTarget::Path(p) });
        }
        out.push(Place { label: t!("shell-filechooser-place-root"), icon: IconName::Disk, target: PlaceTarget::Path(PathBuf::from("/")) });
        self.places = out;
        self.places_devices_and_bookmarks();
    }

    /// Yerler listesinde aygıt ve yer imi gruplarını (yeniden) kurar.
    fn places_devices_and_bookmarks(&mut self) {
        self.places.retain(|p| !matches!(p.target, PlaceTarget::Volume { .. }) && p.icon != IconName::Bookmark);
        for v in &self.volumes {
            self.places.push(Place {
                label: v.display_name(),
                icon: IconName::Usb,
                target: PlaceTarget::Volume { id: v.id.clone(), mount: v.mount_points.first().cloned() },
            });
        }
        for b in places::bookmarks(&self.home) {
            let label = b.label.clone().unwrap_or_else(|| b.path.file_name().map(crate::services::file_chooser::listing::display_name).unwrap_or_else(|| "/".into()));
            self.places.push(Place { label, icon: IconName::Bookmark, target: PlaceTarget::Path(b.path) });
        }
    }

    fn place_items(&self, theme: &nuxut::Theme) -> (Vec<ListItem>, Vec<(usize, String)>) {
        let items = self.places.iter().map(|p| ListItem::new(theme, p.label.clone()).with_leading(theme, Icon::named(p.icon))).collect();
        let mut groups = vec![(0, t!("shell-filechooser-group-places"))];
        if let Some(i) = self.places.iter().position(|p| matches!(p.target, PlaceTarget::Volume { .. })) {
            groups.push((i, t!("shell-filechooser-group-devices")));
        }
        if let Some(i) = self.places.iter().position(|p| p.icon == IconName::Bookmark) {
            groups.push((i, t!("shell-filechooser-group-bookmarks")));
        }
        (items, groups)
    }

    fn open_place(&mut self, cx: &mut Context<Self>, index: usize) {
        let Some(p) = self.places.get(index).cloned() else { return };
        match p.target {
            PlaceTarget::Path(path) => self.navigate(cx, path, true),
            PlaceTarget::Volume { mount: Some(m), .. } => self.navigate(cx, m, true),
            PlaceTarget::Volume { id, mount: None } => {
                if let Some(media) = &self.media
                    && media.open(&id)
                {
                    self.pending_volume = Some(id);
                    self.set_status(cx, Some(t!("shell-filechooser-mounting")));
                }
            }
        }
    }

    // --- Gezinme -------------------------------------------------------------------------

    fn navigate(&mut self, cx: &mut Context<Self>, dir: PathBuf, record: bool) {
        if record && dir != self.dir {
            self.back_stack.push(std::mem::replace(&mut self.dir, dir));
            self.forward_stack.clear();
        } else {
            self.dir = dir;
        }
        self.load(cx);
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.state = DirState::Loading;
        self.selected.clear();
        self.status = None;
        if let Some(w) = self.w.as_mut() {
            w.search.input_mut().clear();
        }
        let sender = cx.sender();
        self.lister.list(self.dir.clone(), move |r| {
            let _ = sender.send(Message::Listed(r));
        });
        self.refresh(cx);
    }

    fn go_up(&mut self, cx: &mut Context<Self>) {
        if let Some(parent) = self.dir.parent().map(Path::to_path_buf) {
            let child = self.dir.file_name().map(OsStr::to_os_string);
            self.navigate(cx, parent, true);
            self.pending_select = child;
        }
    }

    fn go_back(&mut self, cx: &mut Context<Self>) {
        if let Some(prev) = self.back_stack.pop() {
            self.forward_stack.push(std::mem::replace(&mut self.dir, prev));
            self.load(cx);
        }
    }

    fn go_forward(&mut self, cx: &mut Context<Self>) {
        if let Some(next) = self.forward_stack.pop() {
            self.back_stack.push(std::mem::replace(&mut self.dir, next));
            self.load(cx);
        }
    }

    fn listed(&mut self, cx: &mut Context<Self>, r: ListResult) {
        if r.generation != self.lister.current() {
            return;
        }
        match r.result {
            Ok(l) => {
                if l.truncated {
                    self.status = Some(t!("shell-filechooser-truncated"));
                }
                self.state = DirState::Ready(l);
            }
            Err(ListError::NotADirectory) if self.location_pending_file() => return self.select_parent_file(cx),
            Err(e) => self.state = DirState::Failed(e),
        }
        if let Some(name) = self.pending_select.take() {
            self.selected = vec![name];
        }
        // `refresh` seçili adı yeni öğelerde seçer ve görünür kılar.
        self.refresh(cx);
    }

    /// Konum alanından verilen yol bir dosya çıktı mı (Open/Save kipinde üst klasörde seçilir).
    fn location_pending_file(&self) -> bool {
        self.dir.parent().is_some()
    }

    fn select_parent_file(&mut self, cx: &mut Context<Self>) {
        let name = self.dir.file_name().map(OsStr::to_os_string);
        let parent = self.dir.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("/"));
        if self.req().mode == Mode::Save
            && let (Some(n), Some(w)) = (&name, self.w.as_mut())
        {
            w.name.set_text(&n.to_string_lossy());
        }
        self.dir = parent;
        self.pending_select = name;
        self.load(cx);
    }

    // --- Görünür satırlar ----------------------------------------------------------------

    fn listing(&self) -> Option<&Listing> {
        match &self.state {
            DirState::Ready(l) => Some(l),
            _ => None,
        }
    }

    fn entry_visible(&self, e: &Entry, query: &str) -> bool {
        if !self.show_hidden && e.is_hidden() {
            return false;
        }
        if !search_matches(query, &e.display) {
            return false;
        }
        let r = self.req();
        if e.is_dir() {
            return true;
        }
        if r.mode == Mode::SaveFiles || (r.mode == Mode::Open && r.directory) {
            return false;
        }
        match self.filter.and_then(|i| r.filters.get(i)) {
            Some(f) => self.mime.filter_matches(f, &e.display),
            None => true,
        }
    }

    fn row_of(&self, name: &OsStr) -> Option<usize> {
        let l = self.listing()?;
        self.visible.iter().position(|&i| l.entries[i].name == name)
    }

    /// Seçim kipi: yalnız `Open` + `multiple` çoklu.
    fn multiple(&self) -> bool {
        self.req().mode == Mode::Open && self.req().multiple
    }

    /// Görünür satırları ve iki görünümün öğelerini yeniden kurar. Seçim yeni öğelerde sıfırlanır;
    /// önceki seçimin ilk adı hâlâ görünüyorsa o seçili ve görünür kalır (çoklu seçim yeni
    /// içerikte korunmaz: framework görünümleri küme kurmayı sunmuyor, ARCHITECTURE talep).
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(mut w) = self.w.take() else { return };
        let theme = cx.theme().clone();
        let now = Instant::now();
        let query = w.search.text().to_owned();
        self.visible = match self.listing() {
            Some(l) => l.entries.iter().enumerate().filter(|(_, e)| self.entry_visible(e, &query)).map(|(i, _)| i).collect(),
            None => Vec::new(),
        };
        let mut items = Vec::with_capacity(self.visible.len());
        let mut cells = Vec::with_capacity(self.visible.len());
        if let Some(l) = self.listing() {
            for &i in &self.visible {
                let e = &l.entries[i];
                let icon = Icon::named(icon_for(e, &self.mime));
                items.push(ListItem::new(&theme, e.display.clone()).with_leading(&theme, icon.clone()).with_trailing(&theme, ListTrailing::Text(entry_meta(e))));
                cells.push(FileItem::new(e.display.clone(), icon));
            }
        }
        let keep = self.selected.first().and_then(|n| self.row_of(n));
        let files_focused = self.focus == Focus::Files;
        w.files = List::new(&theme).with_density(ListDensity::Compact).with_items(items).with_selection(self.multiple());
        w.grid.set_items(cells, now);
        w.grid.set_focus_visible(files_focused);
        self.thumb_gen = self.thumb_gen.wrapping_add(1);
        self.thumb_seen.clear();
        self.thumb_ready.clear();
        self.thumb_range = 0..0;
        if let Some(t) = &self.thumbs {
            t.request(self.thumb_gen, Vec::new());
        }
        self.selected.clear();
        if let Some(row) = keep {
            w.files.select_only(row);
            w.grid.select_only(row, now);
            if let Some(e) = self.listing().and_then(|l| l.entries.get(self.visible[row])) {
                self.selected.push(e.name.clone());
            }
        }
        if !files_focused {
            w.files.blur(now);
        }
        self.w = Some(w);
        self.refresh_chrome(cx);
    }

    /// Dosya öğelerine dokunmadan çevreyi eşler: yerler vurgusu, yol çubuğu, düğmeler, boş durum,
    /// kabul ve durum satırı.
    fn refresh_chrome(&mut self, cx: &mut Context<Self>) {
        let Some(mut w) = self.w.take() else { return };
        let theme = cx.theme().clone();
        let query = w.search.text().to_owned();
        for (i, item) in w.places.items_mut().iter_mut().enumerate() {
            let here = matches!(self.places.get(i).map(|p| &p.target), Some(PlaceTarget::Path(p)) if *p == self.dir)
                || matches!(self.places.get(i).map(|p| &p.target), Some(PlaceTarget::Volume { mount: Some(m), .. }) if *m == self.dir);
            item.set_selected(here);
        }
        self.update_empty(&theme, &mut w, &query);
        let (segments, targets) = self.path_segments();
        self.path_targets = targets;
        w.path.set_segments(&theme, segments);
        let now = Instant::now();
        let can_up = self.dir.parent().is_some();
        w.up.set_enabled(can_up, now);
        w.back.set_enabled(!self.back_stack.is_empty(), now);
        w.hidden.set_selected(self.show_hidden, now);
        self.w = Some(w);
        self.sync_accept();
        self.sync_status();
        self.redraw(cx);
    }

    fn update_empty(&self, theme: &nuxut::Theme, w: &mut Widgets, query: &str) {
        let (title, mood, icon) = match &self.state {
            DirState::Loading => (String::new(), None, None),
            DirState::Failed(ListError::PermissionDenied) => (t!("shell-filechooser-error-denied"), Some(Mood::Error), Some(IconName::Lock)),
            DirState::Failed(ListError::NotFound) => (t!("shell-filechooser-error-missing"), Some(Mood::Error), Some(IconName::Warning)),
            DirState::Failed(_) => (t!("shell-filechooser-error-read"), Some(Mood::Error), Some(IconName::Warning)),
            DirState::Ready(_) if !self.visible.is_empty() => (String::new(), None, None),
            DirState::Ready(_) if !query.is_empty() => (t!("shell-filechooser-no-results"), Some(Mood::NoResults), Some(IconName::Search)),
            DirState::Ready(_) => (t!("shell-filechooser-empty"), Some(Mood::Empty), Some(IconName::FolderOpen)),
        };
        let mut e = EmptyState::new(theme, title);
        if let Some(m) = mood {
            e = e.with_mascot(theme, m);
        }
        if let Some(i) = icon {
            e = e.with_fallback_icon(theme, Icon::named(i));
        }
        w.empty = e;
    }

    // --- Seçim ---------------------------------------------------------------------------

    fn entry_at(&self, row: usize) -> Option<&Entry> {
        let l = self.listing()?;
        l.entries.get(*self.visible.get(row)?)
    }

    fn selectable(&self, e: &Entry) -> bool {
        let r = self.req();
        match r.mode {
            Mode::Open if r.directory => e.is_dir(),
            Mode::Open => e.is_regular() || (r.multiple && e.is_dir()),
            Mode::Save => e.is_regular(),
            Mode::SaveFiles => e.is_dir(),
        }
    }

    /// Etkin görünümün seçili satırları (artan).
    fn widget_rows(&self) -> Vec<usize> {
        let Some(w) = self.w.as_ref() else { return Vec::new() };
        match self.view_mode {
            ViewMode::List => w.files.selection().map(|s| s.iter().collect()).unwrap_or_default(),
            ViewMode::Grid => w.grid.selection().iter().collect(),
        }
    }

    /// Görünümün seçimi değişti: adları ondan türetir; kaydetmede seçilen dosyanın adı ad alanına
    /// yazılır. Diğer görünüm seçimi izler (çoklu seçim tek öğeye iner: framework küme kurmayı
    /// sunmuyor).
    fn pull_selection(&mut self) {
        let rows = self.widget_rows();
        self.selected = rows.iter().filter_map(|&r| self.entry_at(r).map(|e| e.name.clone())).collect();
        if self.req().mode == Mode::Save
            && let [row] = rows[..]
            && let Some(display) = self.entry_at(row).filter(|e| e.is_regular()).map(|e| e.display.clone())
            && let Some(w) = self.w.as_mut()
        {
            w.name.set_text(&display);
        }
        let first = rows.first().copied();
        let now = Instant::now();
        if let Some(w) = self.w.as_mut() {
            match (self.view_mode, first) {
                (ViewMode::List, Some(r)) => w.grid.select_only(r, now),
                (ViewMode::List, None) => w.grid.clear_selection(),
                (ViewMode::Grid, Some(r)) => w.files.select_only(r),
                (ViewMode::Grid, None) => {}
            }
        }
        self.status = None;
        self.sync_accept();
        self.sync_status();
    }

    /// Görünümün `Activated` olayı (çift tık / Enter): çoklu seçimde kabul, değilse satırı aç.
    fn activated(&mut self, cx: &mut Context<Self>, row: usize) {
        if self.selected.len() > 1 { self.accept(cx) } else { self.activate_row(cx, row) }
    }

    /// Satırı etkinleştir (çift tık / Enter): klasöre gir ya da seç.
    fn activate_row(&mut self, cx: &mut Context<Self>, row: usize) {
        let Some(e) = self.entry_at(row).cloned() else { return };
        if e.is_dir() && !(self.req().mode == Mode::Open && self.req().directory && self.mods.ctrl) {
            let path = self.dir.join(&e.name);
            return self.navigate(cx, path, true);
        }
        if e.is_regular() {
            self.selected = vec![e.name.clone()];
            if self.req().mode == Mode::Save
                && let Some(w) = self.w.as_mut()
            {
                w.name.set_text(&e.display);
            }
            self.accept(cx);
        }
    }

    fn selected_entries(&self) -> Vec<&Entry> {
        let Some(l) = self.listing() else { return Vec::new() };
        self.selected.iter().filter_map(|n| l.entries.iter().find(|e| e.name == *n)).collect()
    }

    fn exists_in_listing(&self, name: &OsStr) -> Option<&Entry> {
        self.listing()?.entries.iter().find(|e| e.name == name)
    }

    // --- Kabul ---------------------------------------------------------------------------

    fn name_error(&self) -> Option<NameError> {
        let w = self.w.as_ref()?;
        check_name(w.name.text()).err()
    }

    fn can_accept(&self) -> bool {
        let r = self.req();
        match r.mode {
            Mode::Open if r.directory => matches!(self.state, DirState::Ready(_)),
            Mode::Open => self.selected_entries().iter().any(|e| self.selectable(e)),
            Mode::Save => self.name_error().is_none() && matches!(self.state, DirState::Ready(_)),
            Mode::SaveFiles => matches!(self.state, DirState::Ready(_)) && !r.files.is_empty(),
        }
    }

    fn sync_accept(&mut self) {
        let ok = self.can_accept();
        if let Some(w) = self.w.as_mut() {
            w.accept.set_enabled(ok, Instant::now());
        }
    }

    fn accept(&mut self, cx: &mut Context<Self>) {
        let r = self.req().clone();
        match r.mode {
            Mode::Open if r.directory => {
                let dirs: Vec<PathBuf> = self.selected_entries().iter().filter(|e| e.is_dir()).map(|e| self.dir.join(&e.name)).collect();
                let paths = if dirs.is_empty() || (!r.multiple && dirs.len() > 1) { vec![self.dir.clone()] } else { dirs };
                self.finish(cx, paths);
            }
            Mode::Open => {
                let entries = self.selected_entries();
                // Tek klasör seçiliyse: içine gir.
                if entries.len() == 1 && entries[0].is_dir() {
                    let path = self.dir.join(&entries[0].name);
                    return self.navigate(cx, path, true);
                }
                let paths: Vec<PathBuf> = entries.iter().filter(|e| e.is_regular()).map(|e| self.dir.join(&e.name)).collect();
                if !paths.is_empty() {
                    self.finish(cx, paths);
                }
            }
            Mode::Save => {
                let Some(name) = self.w.as_ref().map(|w| w.name.text().to_owned()) else { return };
                if let Err(e) = check_name(&name) {
                    return self.set_status(cx, Some(name_error_text(e)));
                }
                let path = self.dir.join(&name);
                match self.exists_in_listing(OsStr::new(&name)) {
                    Some(e) if e.is_dir() => self.navigate(cx, path, true),
                    Some(_) => self.open_overwrite(cx, path),
                    None => self.finish(cx, vec![path]),
                }
            }
            Mode::SaveFiles => {
                let target = match self.selected_entries().as_slice() {
                    [e] if e.is_dir() => self.dir.join(&e.name),
                    _ => self.dir.clone(),
                };
                let in_current = target == self.dir;
                let existing: Vec<OsString> = if in_current { self.listing().map(|l| l.entries.iter().map(|e| e.name.clone()).collect()).unwrap_or_default() } else { Vec::new() };
                if !in_current {
                    // Alt klasörün içeriği okunmadı: önce oraya gidilir, kullanıcı tekrar kabul eder.
                    return self.navigate(cx, target, true);
                }
                let exists = |p: &Path| p.file_name().is_some_and(|n| existing.iter().any(|e| e == n));
                match save_files_targets(&target, &r.files, &exists) {
                    Some(paths) if paths.len() == r.files.len() => self.finish(cx, paths),
                    _ => {
                        // Geçersiz ad (`../x`, `/abs`…): istek tümden reddedilir, hiçbir yol dönmez.
                        tracing::warn!("SaveFiles: geçersiz dosya adı; istek reddedildi");
                        self.cancel(cx);
                    }
                }
            }
        }
    }

    fn open_overwrite(&mut self, cx: &mut Context<Self>, path: PathBuf) {
        let theme = cx.theme().clone();
        let name = path.file_name().map(crate::services::file_chooser::listing::display_name).unwrap_or_default();
        let bounds = Rect::new(0.0, 0.0, self.size.width, self.size.height);
        let Some(w) = self.w.as_mut() else { return };
        w.dialog_body = Label::new(t!("shell-filechooser-overwrite-body", name = name), text_style(&theme, TextRole::Body));
        let width = theme.space(100).min(self.size.width - theme.space(16));
        w.dialog_body.set_max_width(Some(width));
        let body = w.dialog_body.measure(cx.text());
        let actions = vec![
            DialogAction::new(t!("shell-filechooser-cancel"), DialogActionKind::Default),
            DialogAction::new(t!("shell-filechooser-overwrite"), DialogActionKind::Destructive),
        ];
        w.dialog.open(cx.text(), &theme, &t!("shell-filechooser-overwrite-title"), actions, Size::new(width, body.height), bounds, Instant::now());
        self.modal = Modal::Overwrite(path);
        self.redraw(cx);
    }

    fn open_new_folder(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.state, DirState::Ready(_)) {
            return;
        }
        let theme = cx.theme().clone();
        let bounds = Rect::new(0.0, 0.0, self.size.width, self.size.height);
        let now = Instant::now();
        let Some(w) = self.w.as_mut() else { return };
        w.folder_name = TextInput::new(&theme).with_placeholder(t!("shell-filechooser-new-folder-placeholder")).with_max_bytes(255);
        w.folder_name.set_focused(true, now);
        w.name.set_focused(false, now);
        w.search.set_focused(false, now);
        let width = theme.space(90).min(self.size.width - theme.space(16));
        let h = w.folder_name.preferred_size(&theme, width).height;
        let actions = vec![
            DialogAction::new(t!("shell-filechooser-cancel"), DialogActionKind::Default),
            DialogAction::new(t!("shell-filechooser-create"), DialogActionKind::Primary),
        ];
        w.dialog.open(cx.text(), &theme, &t!("shell-filechooser-new-folder"), actions, Size::new(width, h), bounds, now);
        self.modal = Modal::NewFolder;
        self.sync_ime(cx);
        self.redraw(cx);
    }

    fn create_folder(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.w.as_ref().map(|w| w.folder_name.text().to_owned()) else { return };
        if let Err(e) = check_name(&name) {
            return self.set_status(cx, Some(name_error_text(e)));
        }
        let path = self.dir.join(&name);
        let sender = cx.sender();
        let spawned = std::thread::Builder::new().name("nuxut-fc-mkdir".into()).spawn(move || {
            let r = std::fs::create_dir(&path).map(|()| path.file_name().map(OsStr::to_os_string).unwrap_or_default()).map_err(|e| e.kind());
            let _ = sender.send(Message::FolderCreated(r));
        });
        if spawned.is_err() {
            self.set_status(cx, Some(t!("shell-filechooser-error-create")));
        }
    }

    fn close_modal(&mut self, cx: &mut Context<Self>) {
        self.modal = Modal::None;
        if let Some(w) = self.w.as_mut() {
            w.dialog.close(Instant::now());
        }
        let focus = self.focus;
        self.sync_focus(cx, focus);
        self.redraw(cx);
    }

    fn set_status(&mut self, cx: &mut Context<Self>, text: Option<String>) {
        self.status = text;
        self.sync_status();
        self.redraw(cx);
    }

    fn sync_status(&mut self) {
        let text = self.status.clone().or_else(|| {
            if self.req().mode == Mode::Save && self.w.as_ref().is_some_and(|w| !w.name.text().is_empty()) {
                self.name_error().map(name_error_text)
            } else {
                None
            }
        });
        if let Some(w) = self.w.as_mut() {
            w.status.set_text(text.as_deref().unwrap_or(""));
        }
    }

    // --- Bileşenler ----------------------------------------------------------------------

    fn build_widgets(&mut self, cx: &mut Context<Self>) {
        let theme = cx.theme().clone();
        let now = Instant::now();
        let old = self.w.take();
        let keep = |f: &dyn Fn(&Widgets) -> String| old.as_ref().map(f).unwrap_or_default();
        let search_text = keep(&|w| w.search.text().to_owned());
        let name_text = old.as_ref().map_or_else(|| self.req().current_name.clone().unwrap_or_default(), |w| w.name.text().to_owned());

        let mut search = SearchField::new(&theme, &t!("shell-filechooser-search"));
        search.input_mut().set_text(&search_text);
        let (place_items, groups) = self.place_items(&theme);
        let places = List::new(&theme).with_items(place_items).with_groups(&theme, &groups).with_density(ListDensity::Compact);
        let files = List::new(&theme).with_density(ListDensity::Compact).with_selection(self.multiple());
        let filter = (!self.req().filters.is_empty() && self.req().mode != Mode::SaveFiles).then(|| {
            let names = self.req().filters.iter().map(|f| if f.name.is_empty() { t!("shell-filechooser-filter-unnamed") } else { f.name.clone() }).collect();
            Select::new(&theme, names, self.filter.unwrap_or(0))
        });
        let mut choice_labels = Vec::new();
        let mut choices = Vec::new();
        for (c, v) in self.req().choices.iter().zip(&self.choice_values) {
            if c.is_toggle() {
                choice_labels.push(Label::new("", text_style(&theme, TextRole::Label)));
                choices.push(ChoiceWidget::Toggle(Box::new(Chip::new(&theme, ChipKind::Filter, &c.label).selected(v == "true"))));
            } else {
                choice_labels.push(Label::new(c.label.clone(), text_style(&theme, TextRole::Label)));
                let labels = c.options.iter().map(|(_, l)| l.clone()).collect();
                let sel = c.options.iter().position(|(id, _)| id == v).unwrap_or(0);
                choices.push(ChoiceWidget::List(Box::new(Select::new(&theme, labels, sel))));
            }
        }
        let mut name = TextInput::new(&theme).with_placeholder(t!("shell-filechooser-name-placeholder")).with_max_bytes(255).with_text(&name_text);
        name.set_focused(self.focus == Focus::Name, now);
        // Kaydetmede ilk açılışta uzantıdan önceki gövde seçili: yazmak adı değiştirir, uzantı kalır
        // (GTK/KDE davranışı). Yeniden kurulumda (dil/tema) kullanıcının imleci bozulmasın diye yalnız
        // ilk kez.
        if old.is_none() && self.req().mode == Mode::Save {
            let stem = crate::services::file_chooser::names::stem_range(&name_text);
            name.select_range(stem.start, stem.end);
        }
        search.set_focused(self.focus == Focus::Search, now);
        let mut w = Widgets {
            back: IconButton::new(&theme, Icon::named(IconName::ArrowBack), IconButtonVariant::Standard),
            up: IconButton::new(&theme, Icon::named(IconName::ArrowUpward), IconButtonVariant::Standard),
            view: IconButton::new(&theme, Icon::named(self.view_icon()), IconButtonVariant::Standard),
            hidden: IconButton::new(&theme, Icon::named(IconName::Visibility), IconButtonVariant::Standard).toggleable(self.show_hidden),
            new_folder: IconButton::new(&theme, Icon::named(IconName::CreateNewFolder), IconButtonVariant::Standard),
            path: PathBar::new(&theme, Vec::new()),
            search,
            places,
            files,
            grid: FileGrid::new(&theme, self.multiple()),
            empty: EmptyState::new(&theme, ""),
            name_label: Label::new(t!("shell-filechooser-name"), text_style(&theme, TextRole::Label)),
            name,
            filter,
            choice_labels,
            choices,
            cancel: UiButton::new(&theme, ButtonVariant::Text).with_label(&theme, t!("shell-filechooser-cancel")),
            accept: UiButton::new(&theme, ButtonVariant::Filled).with_label(&theme, self.accept_label()),
            status: Label::new("", text_style(&theme, TextRole::Caption)),
            dialog: Dialog::new(&theme),
            dialog_body: Label::new("", text_style(&theme, TextRole::Body)),
            folder_name: TextInput::new(&theme),
        };
        // Açık bir diyalog dil/tema değişiminde kapanır (içeriği yeniden ölçülmeli).
        if let Some(o) = old
            && self.modal != Modal::None
        {
            drop(o);
            self.modal = Modal::None;
        }
        let set_focus = |l: &mut List, on: bool| {
            if !on {
                l.blur(now);
            }
        };
        set_focus(&mut w.places, self.focus == Focus::Places);
        self.w = Some(w);
        self.refresh(cx);
    }

    fn sync_focus(&mut self, cx: &mut Context<Self>, focus: Focus) {
        self.focus = focus;
        let now = Instant::now();
        if let Some(w) = self.w.as_mut() {
            w.search.set_focused(focus == Focus::Search, now);
            w.name.set_focused(focus == Focus::Name, now);
            if focus != Focus::Files {
                w.files.blur(now);
            }
            w.grid.set_focus_visible(focus == Focus::Files && self.view_mode == ViewMode::Grid);
            if focus != Focus::Places {
                w.places.blur(now);
            }
        }
        self.sync_ime(cx);
        self.redraw(cx);
    }

    /// Odaktaki metin alanı (diyalog açıksa onun alanı).
    fn focused_input(&mut self) -> Option<&mut TextInput> {
        let modal = self.modal.clone();
        let focus = self.focus;
        let w = self.w.as_mut()?;
        match (modal, focus) {
            (Modal::NewFolder, _) => Some(&mut w.folder_name),
            (Modal::Overwrite(_), _) => None,
            (_, Focus::Search) => Some(w.search.input_mut()),
            (_, Focus::Location) => w.path.text_input_mut(),
            (_, Focus::Name) => Some(&mut w.name),
            _ => None,
        }
    }

    fn sync_ime(&mut self, cx: &mut Context<Self>) {
        let Some(surface) = self.window else { return };
        let state = self.focused_input().map(|i| {
            let s = i.ime_state();
            TextInputState {
                cursor_rect: s.cursor_rect,
                surrounding: Some(SurroundingText { text: s.surrounding, cursor: s.cursor, anchor: s.anchor }),
                purpose: TextPurpose::Normal,
                multiline: false,
            }
        });
        cx.set_text_input(surface, state);
    }

    fn toggle_hidden(&mut self, cx: &mut Context<Self>) {
        self.show_hidden = !self.show_hidden;
        self.refresh(cx);
    }

    /// Görünüm düğmesinin ikonu: geçilecek görünüm.
    fn view_icon(&self) -> IconName {
        match self.view_mode {
            ViewMode::List => IconName::Grid,
            ViewMode::Grid => IconName::ViewList,
        }
    }

    /// Liste ↔ ızgara (Ctrl+1 / Ctrl+2 ya da düğme). Seçimin ilk öğesi diğer görünümde de seçili.
    fn set_view(&mut self, cx: &mut Context<Self>, mode: ViewMode) {
        if self.view_mode == mode {
            return;
        }
        self.view_mode = mode;
        let icon = self.view_icon();
        let now = Instant::now();
        let files_focused = self.focus == Focus::Files;
        let first = self.selected.first().and_then(|n| self.row_of(n));
        if let Some(w) = self.w.as_mut() {
            w.view.set_icon(Icon::named(icon));
            match (mode, first) {
                (ViewMode::Grid, Some(r)) => w.grid.select_only(r, now),
                (ViewMode::List, Some(r)) => w.files.select_only(r),
                _ => {}
            }
            w.grid.set_focus_visible(files_focused && mode == ViewMode::Grid);
            if !files_focused || mode == ViewMode::Grid {
                w.files.blur(now);
            }
        }
        // Yeni görünümün seçimi esastır (çoklu seçim ilk öğeye iner).
        self.pull_selection();
        self.thumb_range = 0..0;
        self.redraw(cx);
    }

    /// Izgarada görünen (+ önden yükleme payı) resim dosyalarının küçük resimlerini ister. Yalnız
    /// aralık değişince ya da sonuç gelince; işçi ilk gerektiğinde kurulur.
    fn request_thumbnails(&mut self, cx: &mut Context<Self>, now: Instant) {
        let Some(w) = self.w.as_ref() else { return };
        let r = w.grid.visible_range(now);
        let range = r.start.saturating_sub(THUMB_PREFETCH)..(r.end + THUMB_PREFETCH).min(self.visible.len());
        if range == self.thumb_range {
            return;
        }
        self.thumb_range = range.clone();
        let Some(l) = self.listing() else { return };
        let jobs: Vec<ThumbJob> = range
            .filter(|i| !self.thumb_seen.contains(i))
            .filter_map(|i| {
                let e = l.entries.get(*self.visible.get(i)?)?;
                wants_thumbnail(e, &self.mime).then(|| ThumbJob { index: i, path: self.dir.join(&e.name) })
            })
            .collect();
        if jobs.is_empty() {
            return;
        }
        if self.thumbs.is_none() {
            let theme = cx.theme().clone();
            let edge = (FileGrid::thumbnail_size(&theme) * self.scale).ceil() as u32;
            let sender = cx.sender();
            // ADR-0007: yalnız yalıtılmış işçi; işçi yoksa önizleme yok (süreç içi çözüm kapalı).
            let cfg = nuxut_image_sandbox::SandboxConfig {
                in_process_fallback: false,
                rate_per_sec: 30,
                burst: 60,
                timeout: Duration::from_secs(10),
                ..nuxut_image_sandbox::SandboxConfig::wallpaper()
            };
            let sandbox = std::sync::Arc::new(nuxut_image_sandbox::Sandbox::new(cfg));
            let decoder = crate::services::image_sandbox::isolated(&sandbox);
            match Thumbnailer::new(decoder, edge, move |t| {
                let _ = sender.send(Message::Thumb(t));
            }) {
                Ok(t) => self.thumbs = Some(t),
                Err(e) => {
                    tracing::warn!("küçük resim işçisi başlatılamadı: {e}");
                    return;
                }
            }
        }
        if let Some(t) = &self.thumbs {
            t.request(self.thumb_gen, jobs);
        }
    }

    fn thumb_arrived(&mut self, cx: &mut Context<Self>, t: Thumb) {
        if t.generation != self.thumb_gen {
            return;
        }
        self.thumb_seen.insert(t.index);
        if let Some(image) = t.image {
            self.thumb_ready.push((t.index, image));
            self.redraw(cx);
        }
    }

    fn open_location(&mut self, cx: &mut Context<Self>) {
        self.location_open = true;
        let text = self.dir.to_string_lossy().into_owned();
        if let Some(w) = self.w.as_mut() {
            let t = if text.ends_with('/') { text } else { format!("{text}/") };
            w.path.begin_edit(cx.theme(), &t, Instant::now());
        }
        self.sync_focus(cx, Focus::Location);
    }

    fn close_location(&mut self, cx: &mut Context<Self>) {
        self.location_open = false;
        if let Some(w) = self.w.as_mut() {
            w.path.end_edit();
        }
        self.sync_focus(cx, Focus::Files);
    }

    fn submit_location(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.w.as_ref().and_then(|w| w.path.edit_text()).map(str::to_owned) else { return };
        self.location_open = false;
        if let Some(w) = self.w.as_mut() {
            w.path.end_edit();
        }
        match resolve_location(&text, &self.dir, &self.home) {
            Some(path) => {
                self.sync_focus(cx, Focus::Files);
                self.navigate(cx, path, true);
            }
            None => self.close_location(cx),
        }
    }

    fn path_event(&mut self, cx: &mut Context<Self>, r: nuxut::ui::Response<PathBarEvent>) {
        match r.event {
            Some(PathBarEvent::Navigate(i)) => {
                if let Some(p) = self.path_targets.get(i).cloned() {
                    self.navigate(cx, p, true);
                }
            }
            Some(PathBarEvent::Edit(e)) => {
                let r = nuxut::ui::Response { repaint: r.repaint, event: Some(e) };
                self.text_event(cx, Focus::Location, r);
            }
            None if r.repaint => self.redraw(cx),
            None => {}
        }
    }

    /// Yol çubuğu parçaları: ev dizini altındaysa "Ev"den, değilse kökten.
    fn path_segments(&self) -> (Vec<PathSegment>, Vec<PathBuf>) {
        let (mut segs, mut targets, rest) = match self.dir.strip_prefix(&self.home) {
            Ok(rest) if self.home != Path::new("/") => (
                vec![PathSegment::new(t!("shell-filechooser-place-home")).with_icon(Icon::named(IconName::Home))],
                vec![self.home.clone()],
                rest.to_path_buf(),
            ),
            _ => (
                vec![PathSegment::new(t!("shell-filechooser-place-root")).with_icon(Icon::named(IconName::Disk))],
                vec![PathBuf::from("/")],
                self.dir.strip_prefix("/").map(Path::to_path_buf).unwrap_or_default(),
            ),
        };
        let mut cur = targets[0].clone();
        for c in rest.components() {
            cur.push(c);
            segs.push(PathSegment::new(crate::services::file_chooser::listing::display_name(c.as_os_str())));
            targets.push(cur.clone());
        }
        (segs, targets)
    }

    // --- Girdi ---------------------------------------------------------------------------

    fn text_event(&mut self, cx: &mut Context<Self>, focus: Focus, r: nuxut::ui::Response<TextInputEvent>) {
        match r.event {
            Some(TextInputEvent::Changed) => match focus {
                Focus::Search => self.refresh(cx),
                Focus::Name => {
                    self.status = None;
                    self.sync_accept();
                    self.sync_status();
                }
                _ => {}
            },
            Some(TextInputEvent::Submit) => match (self.modal.clone(), focus) {
                (Modal::NewFolder, _) => self.create_folder(cx),
                (_, Focus::Location) => self.submit_location(cx),
                (_, Focus::Name) => self.accept(cx),
                (_, Focus::Search) => {
                    if let Some(row) = self.selected.first().and_then(|n| self.row_of(n)).or((!self.visible.is_empty()).then_some(0)) {
                        self.activate_row(cx, row);
                    }
                }
                _ => {}
            },
            Some(TextInputEvent::Cancel) => match (self.modal.clone(), focus) {
                (Modal::NewFolder, _) => self.close_modal(cx),
                (_, Focus::Location) => self.close_location(cx),
                (_, Focus::Search) if self.w.as_ref().is_some_and(|w| !w.search.text().is_empty()) => {
                    if let Some(w) = self.w.as_mut() {
                        w.search.input_mut().clear();
                    }
                    self.sync_focus(cx, Focus::Files);
                    self.refresh(cx);
                }
                _ => self.cancel(cx),
            },
            Some(TextInputEvent::Copy(s) | TextInputEvent::Cut(s)) => {
                let _ = cx.set_clipboard(nuxut::clipboard::Selection::Clipboard, &s);
            }
            Some(TextInputEvent::PasteRequested) => {
                self.paste = Some((cx.request_paste(nuxut::clipboard::Selection::Clipboard), focus));
            }
            _ => {}
        }
        self.sync_ime(cx);
        self.redraw(cx);
    }

    fn cycle_focus(&mut self, cx: &mut Context<Self>, back: bool) {
        let mut order = vec![Focus::Places, Focus::Files];
        if self.req().mode == Mode::Save {
            order.push(Focus::Name);
        }
        let cur = order.iter().position(|f| *f == self.focus).unwrap_or(1);
        let next = if back { (cur + order.len() - 1) % order.len() } else { (cur + 1) % order.len() };
        self.sync_focus(cx, order[next]);
    }

    fn key_event(&mut self, cx: &mut Context<Self>, event: &KeyEvent) {
        if event.state == KeyState::Released {
            return;
        }
        self.mods = event.modifiers;
        let now = Instant::now();
        let theme = cx.theme().clone();

        // Diyalog.
        match self.modal.clone() {
            Modal::Overwrite(path) => {
                let r = self.w.as_mut().map(|w| w.dialog.key(event, now));
                if let Some(r) = r {
                    match r.event {
                        Some(DialogEvent::Action(1)) => {
                            self.modal = Modal::None;
                            return self.finish(cx, vec![path]);
                        }
                        Some(_) => return self.close_modal(cx),
                        None if r.repaint => self.redraw(cx),
                        None => {}
                    }
                }
                return;
            }
            Modal::NewFolder => {
                if matches!(event.key, Key::Escape) {
                    return self.close_modal(cx);
                }
                let r = self.w.as_mut().map(|w| w.folder_name.key(event, now));
                if let Some(r) = r {
                    self.text_event(cx, self.focus, r);
                }
                return;
            }
            Modal::None => {}
        }

        // Açık seçim menüleri.
        if let Some(w) = self.w.as_mut() {
            let text = cx.text();
            if let Some(f) = w.filter.as_mut().filter(|f| f.is_open()) {
                let r = f.key(text, &theme, event, now);
                if let Some(SelectEvent::Changed(i)) = r.event {
                    self.filter = Some(i);
                    self.refresh(cx);
                }
                return self.redraw(cx);
            }
            for (i, c) in w.choices.iter_mut().enumerate() {
                if let ChoiceWidget::List(s) = c
                    && s.is_open()
                {
                    let r = s.key(text, &theme, event, now);
                    if let Some(SelectEvent::Changed(o)) = r.event
                        && let Some(id) = self.setup.request.choices.get(i).and_then(|c| c.options.get(o)).map(|(id, _)| id.clone())
                        && let Some(v) = self.choice_values.get_mut(i)
                    {
                        *v = id;
                    }
                    return self.redraw(cx);
                }
            }
        }

        let m = event.modifiers;
        match event.key {
            Key::Char(c) if m.ctrl && c.eq_ignore_ascii_case(&'l') => return self.open_location(cx),
            Key::Char(c) if m.ctrl && c.eq_ignore_ascii_case(&'h') => return self.toggle_hidden(cx),
            Key::Char(c) if m.ctrl && m.shift && c.eq_ignore_ascii_case(&'n') && self.can_create_folder() => return self.open_new_folder(cx),
            // Yerlerdeyken Ctrl+A dosyaları seçer (görünüm seçim kuralları).
            Key::Char(c) if m.ctrl && c.eq_ignore_ascii_case(&'a') && self.focus == Focus::Places => {
                self.sync_focus(cx, Focus::Files);
                return self.files_key(cx, event);
            }
            Key::Char('1') if m.ctrl => return self.set_view(cx, ViewMode::List),
            Key::Char('2') if m.ctrl => return self.set_view(cx, ViewMode::Grid),
            Key::Up if m.alt => return self.go_up(cx),
            Key::Left if m.alt => return self.go_back(cx),
            Key::Right if m.alt => return self.go_forward(cx),
            Key::Other(sym) if sym == XK_F5 => return self.load(cx),
            Key::Tab => return self.cycle_focus(cx, m.shift),
            _ => {}
        }

        match self.focus {
            Focus::Search | Focus::Location | Focus::Name => {
                // Arama alanında ↑↓ listeye geçer.
                if self.focus == Focus::Search && matches!(event.key, Key::Down | Key::Up) {
                    self.sync_focus(cx, Focus::Files);
                    return self.files_key(cx, event);
                }
                let focus = self.focus;
                let r = match (self.w.as_mut(), focus) {
                    (Some(w), Focus::Search) => w.search.key(event, now),
                    (Some(w), Focus::Location) => {
                        let r = w.path.key(event, now);
                        return self.path_event(cx, r);
                    }
                    (Some(w), Focus::Name) => w.name.key(event, now),
                    _ => return,
                };
                self.text_event(cx, focus, r);
            }
            Focus::Files => self.files_key(cx, event),
            Focus::Places => {
                if self.type_to_search(cx, event) {
                    return;
                }
                if event.key == Key::Escape {
                    return self.cancel(cx);
                }
                let Some(w) = self.w.as_mut() else { return };
                let r = w.places.key(&theme, event, now);
                if let Some((i, ListEvent::Clicked)) = r.event {
                    self.open_place(cx, i);
                }
                self.redraw(cx);
            }
        }
    }

    /// Yazdırılabilir karakter: arama alanına geçip yazar.
    fn type_to_search(&mut self, cx: &mut Context<Self>, event: &KeyEvent) -> bool {
        let m = event.modifiers;
        let printable = event.text.as_ref().is_some_and(|t| !t.is_empty()) && !m.ctrl && !m.alt && !m.logo && event.key != Key::Space;
        if !printable {
            return false;
        }
        self.sync_focus(cx, Focus::Search);
        let now = Instant::now();
        if let Some(r) = self.w.as_mut().map(|w| w.search.key(event, now)) {
            self.text_event(cx, Focus::Search, r);
        }
        true
    }

    fn files_key(&mut self, cx: &mut Context<Self>, event: &KeyEvent) {
        if self.type_to_search(cx, event) {
            return;
        }
        let theme = cx.theme().clone();
        let now = Instant::now();
        match event.key {
            Key::Escape => {
                if self.w.as_ref().is_some_and(|w| !w.search.text().is_empty()) {
                    if let Some(w) = self.w.as_mut() {
                        w.search.input_mut().clear();
                    }
                    return self.refresh(cx);
                }
                return self.cancel(cx);
            }
            Key::Backspace => return self.go_up(cx),
            // Seçim yokken de Enter kabul eder (kaydetme, klasör seçimi); görünümün `Activated`'ı
            // yalnız odak satırı varken gelirdi.
            Key::Enter => {
                let focused = match self.view_mode {
                    ViewMode::List => self.w.as_ref().and_then(|w| w.files.focused()),
                    ViewMode::Grid => None,
                };
                let row = focused.or_else(|| self.selected.first().and_then(|n| self.row_of(n)));
                match row {
                    Some(r) => self.activated(cx, r),
                    None => self.accept(cx),
                }
                return;
            }
            _ => {}
        }
        let Some(w) = self.w.as_mut() else { return };
        let (changed, activated, repaint) = match self.view_mode {
            ViewMode::List => {
                let r = w.files.key_select(&theme, event, now);
                match r.event {
                    Some(ListSelectEvent::Changed) => (true, None, true),
                    Some(ListSelectEvent::Activated(i)) => (false, Some(i), true),
                    _ => (false, None, r.repaint),
                }
            }
            ViewMode::Grid => {
                let r = w.grid.key(event, now);
                match r.event {
                    Some(FileGridEvent::SelectionChanged) => (true, None, true),
                    Some(FileGridEvent::Activated(i)) => (false, Some(i), true),
                    None => (false, None, r.repaint),
                }
            }
        };
        if changed {
            self.pull_selection();
        }
        if let Some(i) = activated {
            return self.activated(cx, i);
        }
        if repaint {
            self.redraw(cx);
        }
    }

    fn pointer_event(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: &PointerEvent) {
        let now = Instant::now();
        let theme = cx.theme().clone();
        let Some(mut w) = self.w.take() else { return };
        let mut cursor: Option<CursorShape> = None;
        let mut repaint = false;

        // Diyalog her şeyi alır.
        if self.modal != Modal::None {
            if self.modal == Modal::NewFolder {
                let r = w.folder_name.pointer(cx.text(), &theme, event, now);
                repaint |= r.repaint;
                cursor = w.folder_name.cursor(event.position);
            }
            let r = w.dialog.pointer(event, now);
            repaint |= r.repaint;
            cursor = cursor.or_else(|| w.dialog.cursor(event.position));
            self.w = Some(w);
            cx.set_cursor(surface, cursor.unwrap_or(CursorShape::Default));
            match (self.modal.clone(), r.event) {
                (Modal::Overwrite(path), Some(DialogEvent::Action(1))) => {
                    self.modal = Modal::None;
                    return self.finish(cx, vec![path]);
                }
                (Modal::NewFolder, Some(DialogEvent::Action(1))) => return self.create_folder(cx),
                (_, Some(_)) => return self.close_modal(cx),
                _ => {}
            }
            if repaint {
                self.redraw(cx);
            }
            return;
        }

        // Açık seçim menüsü olayları önce alır.
        let mut select_open = false;
        if let Some(f) = w.filter.as_mut()
            && f.is_open()
        {
            select_open = true;
            let r = f.pointer(cx.text(), &theme, event, now);
            repaint |= r.repaint;
            if let Some(SelectEvent::Changed(i)) = r.event {
                self.filter = Some(i);
                self.w = Some(w);
                self.refresh(cx);
                return;
            }
            cursor = f.cursor(event.position);
        }
        for (i, c) in w.choices.iter_mut().enumerate() {
            if let ChoiceWidget::List(s) = c
                && s.is_open()
            {
                select_open = true;
                let r = s.pointer(cx.text(), &theme, event, now);
                repaint |= r.repaint;
                if let Some(SelectEvent::Changed(o)) = r.event
                    && let Some(id) = self.setup.request.choices.get(i).and_then(|c| c.options.get(o)).map(|(id, _)| id.clone())
                    && let Some(v) = self.choice_values.get_mut(i)
                {
                    *v = id;
                }
                cursor = s.cursor(event.position);
            }
        }
        if select_open {
            self.w = Some(w);
            cx.set_cursor(surface, cursor.unwrap_or(CursorShape::Default));
            if repaint {
                self.redraw(cx);
            }
            return;
        }

        enum Act {
            Back,
            Up,
            Hidden,
            NewFolder,
            Place(usize),
            Segment(usize),
            Row(usize),
            /// Görünümün seçimi değişti.
            Selected,
            View,
            Cancel,
            Accept,
            Focus(Focus),
            Filter(usize),
            Choice(usize, String),
        }
        let mut acts: Vec<Act> = Vec::new();
        let press = matches!(event.kind, PointerKind::Press(_));
        let p = event.position;

        let r = w.back.pointer(event, now);
        repaint |= r.repaint;
        if r.event.is_some() {
            acts.push(Act::Back);
        }
        let r = w.up.pointer(event, now);
        repaint |= r.repaint;
        if r.event.is_some() {
            acts.push(Act::Up);
        }
        let r = w.hidden.pointer(event, now);
        repaint |= r.repaint;
        if r.event.is_some() {
            acts.push(Act::Hidden);
        }
        let r = w.view.pointer(event, now);
        repaint |= r.repaint;
        if r.event.is_some() {
            acts.push(Act::View);
        }
        if self.can_create_folder() {
            let r = w.new_folder.pointer(event, now);
            repaint |= r.repaint;
            if r.event.is_some() {
                acts.push(Act::NewFolder);
            }
        }
        let r = w.search.pointer(cx.text(), &theme, event, now);
        repaint |= r.repaint;
        if press && w.search.rect().contains(p) {
            acts.push(Act::Focus(Focus::Search));
        }
        let r = w.path.pointer(cx.text(), &theme, event, now);
        repaint |= r.repaint;
        match r.event {
            Some(PathBarEvent::Navigate(i)) => acts.push(Act::Segment(i)),
            Some(PathBarEvent::Edit(_)) => acts.push(Act::Focus(Focus::Location)),
            None => {}
        }
        if self.req().mode == Mode::Save {
            let r = w.name.pointer(cx.text(), &theme, event, now);
            repaint |= r.repaint;
            if press && w.name.rect().contains(p) {
                acts.push(Act::Focus(Focus::Name));
            }
        }
        let r = w.places.pointer(&theme, event, now);
        repaint |= r.repaint;
        if let Some((i, ListEvent::Clicked)) = r.event {
            acts.push(Act::Place(i));
        }
        let files_shown = !self.visible.is_empty();
        if files_shown {
            match self.view_mode {
                ViewMode::List => {
                    let r = w.files.pointer_select(&theme, event, self.mods, now);
                    repaint |= r.repaint;
                    match r.event {
                        Some(ListSelectEvent::Changed) => acts.push(Act::Selected),
                        Some(ListSelectEvent::Activated(i)) => acts.push(Act::Row(i)),
                        _ => {}
                    }
                }
                ViewMode::Grid => {
                    let r = w.grid.pointer(&theme, event, self.mods, now);
                    repaint |= r.repaint;
                    match r.event {
                        Some(FileGridEvent::SelectionChanged) => acts.push(Act::Selected),
                        Some(FileGridEvent::Activated(i)) => acts.push(Act::Row(i)),
                        None => {}
                    }
                }
            }
        }
        if let Some(f) = w.filter.as_mut() {
            let r = f.pointer(cx.text(), &theme, event, now);
            repaint |= r.repaint;
            if let Some(SelectEvent::Changed(i)) = r.event {
                acts.push(Act::Filter(i));
            }
        }
        for (i, c) in w.choices.iter_mut().enumerate() {
            match c {
                ChoiceWidget::Toggle(chip) => {
                    let r = chip.pointer(&theme, event, now);
                    repaint |= r.repaint;
                    if let Some(ChipEvent::Toggled(on)) = r.event {
                        acts.push(Act::Choice(i, if on { "true".into() } else { "false".into() }));
                    }
                }
                ChoiceWidget::List(s) => {
                    let r = s.pointer(cx.text(), &theme, event, now);
                    repaint |= r.repaint;
                }
            }
        }
        let r = w.cancel.pointer(event, now);
        repaint |= r.repaint;
        if r.event.is_some() {
            acts.push(Act::Cancel);
        }
        let r = w.accept.pointer(event, now);
        repaint |= r.repaint;
        if r.event.is_some() {
            acts.push(Act::Accept);
        }

        cursor = [
            w.back.cursor(p),
            w.up.cursor(p),
            w.hidden.cursor(p),
            if self.can_create_folder() { w.new_folder.cursor(p) } else { None },
            w.search.cursor(p),
            w.path.cursor(p),
            if self.req().mode == Mode::Save { w.name.cursor(p) } else { None },
            w.places.cursor(p),
            match (files_shown, self.view_mode) {
                (false, _) => None,
                (true, ViewMode::List) => w.files.cursor(p),
                (true, ViewMode::Grid) => w.grid.cursor_shape(p),
            },
            w.view.cursor(p),
            w.filter.as_ref().and_then(|f| f.cursor(p)),
            w.cancel.cursor(p),
            w.accept.cursor(p),
        ]
        .into_iter()
        .flatten()
        .next()
        .or_else(|| {
            w.choices.iter().find_map(|c| match c {
                ChoiceWidget::Toggle(_) => None,
                ChoiceWidget::List(s) => s.cursor(p),
            })
        });
        let places_hit = press && w.places.rect().contains(p);
        let files_rect = if self.view_mode == ViewMode::Grid { w.grid.rect() } else { w.files.rect() };
        let files_hit = press && files_rect.contains(p);
        self.w = Some(w);
        cx.set_cursor(surface, cursor.unwrap_or(CursorShape::Default));
        if places_hit {
            acts.push(Act::Focus(Focus::Places));
        } else if files_hit && !matches!(self.focus, Focus::Files) {
            acts.insert(0, Act::Focus(Focus::Files));
        }

        for a in acts {
            if self.done {
                return;
            }
            match a {
                Act::Back => self.go_back(cx),
                Act::Up => self.go_up(cx),
                Act::Hidden => self.toggle_hidden(cx),
                Act::NewFolder => self.open_new_folder(cx),
                Act::Place(i) => self.open_place(cx, i),
                Act::Segment(i) => {
                    if let Some(p) = self.path_targets.get(i).cloned() {
                        self.navigate(cx, p, true);
                    }
                }
                Act::Row(_) if access::assistive_guard(self.assistive_click, Instant::now()) => {
                    tracing::info!("dosya seçici: AT tıklamasına bağlı çift tık yok sayıldı");
                }
                Act::Row(i) => self.activated(cx, i),
                Act::Selected => self.pull_selection(),
                Act::View => {
                    let next = if self.view_mode == ViewMode::List { ViewMode::Grid } else { ViewMode::List };
                    self.set_view(cx, next);
                }
                Act::Cancel => self.cancel(cx),
                Act::Accept => self.accept(cx),
                Act::Focus(f) => {
                    if f != self.focus {
                        self.sync_focus(cx, f);
                    }
                }
                Act::Filter(i) => {
                    self.filter = Some(i);
                    self.refresh(cx);
                }
                Act::Choice(i, v) => {
                    if let Some(slot) = self.choice_values.get_mut(i) {
                        *slot = v;
                    }
                }
            }
            repaint = true;
        }
        if repaint {
            self.redraw(cx);
        }
    }

    fn can_create_folder(&self) -> bool {
        let r = self.req();
        r.mode != Mode::Open || r.directory
    }

    // --- Çizim ---------------------------------------------------------------------------

    fn draw_window(&mut self, cx: &mut Context<Self>, surface: SurfaceId, now: Instant, painter: &mut Painter) {
        let theme = cx.theme().clone();
        let size = self.size;
        let can_create = self.can_create_folder();
        let save = self.req().mode == Mode::Save;
        let has_items = !self.visible.is_empty();
        let loading = matches!(self.state, DirState::Loading);
        let grid = self.view_mode == ViewMode::Grid;
        let ready = std::mem::take(&mut self.thumb_ready);
        let Some(w) = self.w.as_mut() else { return };
        // Gelen küçük resimler dokuya (painter yalnız çizimde).
        for (i, image) in ready {
            let (iw, ih) = image.size();
            let texture = painter.upload_texture(ImageData { width: iw, height: ih, pixels: image.into_pixels().into() }, TextureOptions::default());
            w.grid.set_thumbnail(i, Some(texture));
        }
        let mut ui = Ui::new(painter, &theme, now);
        let c = theme.colors;
        ui.painter.rect(Rect::new(0.0, 0.0, size.width, size.height), &nuxut::RectStyle::filled(c.surface, nuxut::Corners::ZERO));

        let pad = theme.space(4);
        let gap = theme.space(2);
        let row_h = theme.space(10);

        // Üst satır.
        let mut x = pad;
        let y = pad;
        for b in [&mut w.back, &mut w.up] {
            let s = b.preferred_size(&theme);
            b.set_rect(Rect::new(x, y + (row_h - s.height) * 0.5, s.width, s.height));
            b.draw(&mut ui);
            x += s.width + theme.space(1);
        }
        x += gap;
        let search_w = theme.space(60).min(size.width * 0.3);
        let mut right = size.width - pad - search_w;
        let sh = w.search.preferred_size(&theme, search_w).height;
        w.search.set_rect(&theme, Rect::new(right, y + (row_h - sh) * 0.5, search_w, sh));
        w.search.draw(&mut ui);
        right -= gap;
        let mut icons: Vec<&mut IconButton> = vec![&mut w.hidden, &mut w.view];
        if can_create {
            icons.push(&mut w.new_folder);
        }
        for b in icons {
            let s = b.preferred_size(&theme);
            right -= s.width;
            b.set_rect(Rect::new(right, y + (row_h - s.height) * 0.5, s.width, s.height));
            b.draw(&mut ui);
            right -= theme.space(1);
        }
        let title_w = (right - gap - x).max(0.0);
        let ph = w.path.preferred_size(&theme, title_w).height;
        w.path.set_rect(Rect::new(x, y + (row_h - ph) * 0.5, title_w, ph));
        w.path.draw(&mut ui);

        // Alt bölüm (aşağıdan yukarı).
        let mut bottom = size.height - pad;
        let buttons_y = bottom - row_h;
        let mut bx = size.width - pad;
        for b in [&mut w.accept, &mut w.cancel] {
            let s = b.preferred_size(ui.painter.text_system(), &theme);
            bx -= s.width;
            b.set_rect(Rect::new(bx, buttons_y + (row_h - s.height) * 0.5, s.width, s.height));
            bx -= gap;
        }
        // Seçim kutuları: düğmelerin solunda, soldan.
        let mut cx_ = pad;
        for (label, c_) in w.choice_labels.iter_mut().zip(w.choices.iter_mut()) {
            if !label.text().is_empty() {
                let s = label.measure_in(ui.painter);
                label.draw(ui.painter, Point::new(cx_, buttons_y + (row_h - s.height) * 0.5), ui.fade(c.on_surface_variant));
                cx_ += s.width + theme.space(2);
            }
            match c_ {
                ChoiceWidget::Toggle(chip) => {
                    let s = chip.preferred_size(ui.painter.text_system(), &theme);
                    chip.set_rect(Rect::new(cx_, buttons_y + (row_h - s.height) * 0.5, s.width, s.height));
                    cx_ += s.width + gap;
                }
                ChoiceWidget::List(sel) => {
                    let width = theme.space(40);
                    let s = sel.preferred_size(&theme, width);
                    sel.set_rect(&theme, Rect::new(cx_, buttons_y + (row_h - s.height) * 0.5, width, s.height));
                    sel.set_bounds(Rect::new(0.0, 0.0, size.width, size.height));
                    cx_ += width + gap;
                }
            }
        }
        bottom = buttons_y - gap;
        if save || w.filter.is_some() {
            let ny = bottom - row_h;
            let mut fx = size.width - pad;
            if let Some(f) = w.filter.as_mut() {
                let width = theme.space(56).min(size.width * 0.35);
                let s = f.preferred_size(&theme, width);
                fx -= width;
                f.set_rect(&theme, Rect::new(fx, ny + (row_h - s.height) * 0.5, width, s.height));
                f.set_bounds(Rect::new(0.0, 0.0, size.width, size.height));
                fx -= gap;
            }
            if save {
                let ls = w.name_label.measure_in(ui.painter);
                w.name_label.draw(ui.painter, Point::new(pad, ny + (row_h - ls.height) * 0.5), ui.fade(c.on_surface_variant));
                let nx = pad + ls.width + theme.space(3);
                let nw = (fx - nx).max(theme.space(20));
                let h = w.name.preferred_size(&theme, nw).height;
                w.name.set_rect(Rect::new(nx, ny + (row_h - h) * 0.5, nw, h));
                w.name.draw(&mut ui);
            }
            bottom = ny - gap;
        }
        // Durum satırı.
        let ss = w.status.measure_in(ui.painter);
        if !w.status.text().is_empty() {
            w.status.draw(ui.painter, Point::new(pad, bottom - ss.height), ui.fade(c.error));
        }
        bottom -= ss.height.max(theme.space(4)) + theme.space(1);

        // Gövde: yerler + dosyalar.
        let body_top = y + row_h + gap;
        let body_h = (bottom - body_top).max(0.0);
        let side_w = theme.space(56).min(size.width * 0.3);
        let side = Rect::new(pad, body_top, side_w, body_h);
        w.places.set_rect(&theme, side, now);
        w.places.draw(&mut ui);
        let files_rect = Rect::new(pad + side_w + gap, body_top, (size.width - 2.0 * pad - side_w - gap).max(0.0), body_h);
        ui.painter.rect(files_rect, &nuxut::RectStyle::filled(c.surface_container, nuxut::Corners::uniform(theme.shape.radius_medium)));
        let inner = files_rect.inset(nuxut::Insets::uniform(theme.space(1)));
        if has_items && grid {
            w.grid.set_rect(&theme, inner, now);
            w.grid.draw(&mut ui);
        } else if has_items {
            w.files.set_rect(&theme, inner, now);
            w.files.draw(&mut ui);
        } else if !loading {
            w.empty.set_rect(inner);
            w.empty.draw(&mut ui);
        }

        // Düğmeler ve seçim kutuları (menüler en üstte).
        w.cancel.draw(&mut ui);
        w.accept.draw(&mut ui);
        for c_ in &mut w.choices {
            match c_ {
                ChoiceWidget::Toggle(chip) => chip.draw(&mut ui),
                ChoiceWidget::List(sel) => sel.draw(&mut ui),
            }
        }
        if let Some(f) = w.filter.as_mut() {
            f.draw(&mut ui);
        }
        for c_ in &mut w.choices {
            if let ChoiceWidget::List(sel) = c_ {
                sel.draw_overlay(&mut ui);
            }
        }
        if let Some(f) = w.filter.as_mut() {
            f.draw_overlay(&mut ui);
        }

        // Diyalog.
        let modal = self.modal.clone();
        if w.dialog.is_visible(now) {
            let (body, input) = (&mut w.dialog_body, &mut w.folder_name);
            w.dialog.draw(&mut ui, |ui, rect| match modal {
                Modal::NewFolder => {
                    input.set_rect(rect);
                    input.draw(ui);
                }
                _ => body.draw(ui.painter, Point::new(rect.x, rect.y), ui.fade(ui.theme.colors.on_surface)),
            });
        }

        if ui.wants_frame() {
            cx.request_frame(surface);
        }
        let deadline = ui.wake_deadline();
        drop(ui);
        if let Some(t) = self.wake.take() {
            cx.cancel_timer(t);
        }
        if let Some(at) = deadline {
            self.wake = Some(cx.set_timer(at.saturating_duration_since(now)));
        }
        // Izgarada görünen hücrelerin küçük resimleri (aralık değiştiyse).
        if grid && has_items {
            self.request_thumbnails(cx, now);
        }
    }

    fn media_event(&mut self, cx: &mut Context<Self>, e: MediaEvent) {
        match e {
            MediaEvent::Volumes { volumes, .. } => {
                self.volumes = volumes;
                self.places_devices_and_bookmarks();
                let theme = cx.theme().clone();
                let (items, groups) = self.place_items(&theme);
                if let Some(w) = self.w.as_mut() {
                    *w.places.items_mut() = items;
                    w.places.set_groups(&theme, &groups);
                }
                self.refresh_chrome(cx);
            }
            MediaEvent::Opened { id, mount_point } if self.pending_volume.as_deref() == Some(id.as_str()) => {
                self.pending_volume = None;
                self.navigate(cx, mount_point, true);
            }
            MediaEvent::Failed { id, error, .. } if self.pending_volume.as_deref() == Some(id.as_str()) => {
                self.pending_volume = None;
                if !error.silent() {
                    self.set_status(cx, Some(error.message()));
                } else {
                    self.set_status(cx, None);
                }
            }
            _ => {}
        }
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        self.started = true;
        // Arka plan işleri run başladıktan sonra (sinyal maskesi).
        self.mime = MimeDb::load();
        self.build_places();
        if self.setup.watch_stdin {
            let sender = cx.sender();
            let _ = std::thread::Builder::new().name("nuxut-fc-stdin".into()).spawn(move || {
                use std::io::Read;
                let mut buf = [0u8; 256];
                let mut stdin = std::io::stdin().lock();
                loop {
                    match stdin.read(&mut buf) {
                        Ok(0) => break,
                        Ok(_) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
                let _ = sender.send(Message::Lifeline);
            });
        }
        let sender = cx.sender();
        let options = MediaOptions { activate: false, ..MediaOptions::default() };
        match MediaService::spawn(options, move |e| {
            let _ = sender.send(Message::Media(e));
        }) {
            Ok(m) => self.media = Some(m),
            Err(e) => tracing::warn!("çıkarılabilir medya izlenemiyor: {e}"),
        }
        let spec = WindowSpec { size: WINDOW_SIZE, min_size: Some(MIN_SIZE), ..WindowSpec::new(self.title(), APP_ID) };
        self.window = Some(cx.create_window(spec));
        self.build_widgets(cx);
        self.load(cx);
    }
}

/// XKB `F5` keysym.
const XK_F5: u32 = 0xffc2;

fn name_error_text(e: NameError) -> String {
    match e {
        NameError::Empty => t!("shell-filechooser-name-empty"),
        NameError::Dot => t!("shell-filechooser-name-dot"),
        NameError::Slash => t!("shell-filechooser-name-slash"),
        NameError::TooLong => t!("shell-filechooser-name-long"),
    }
}

impl App for FileChooser {
    type Message = Message;

    fn output_added(&mut self, cx: &mut Context<Self>, _output: &OutputInfo) {
        if !self.started {
            self.start(cx);
        }
    }

    fn output_removed(&mut self, _cx: &mut Context<Self>, _output: OutputId) {}

    fn surface_resized(&mut self, cx: &mut Context<Self>, surface: SurfaceId, size: Size, scale: f32) {
        if Some(surface) == self.window {
            if !self.parent_set && !self.setup.request.parent.is_empty() {
                // İlk configure sonrası: ebeveyn (xdg-foreign). Tanıtıcı loglanmaz.
                self.parent_set = true;
                let ok = cx.set_window_parent(surface, &self.setup.request.parent);
                tracing::info!(parent = ok, "ebeveyn pencere");
            }
            self.size = size;
            self.scale = scale.max(1.0);
            cx.request_frame(surface);
        }
    }

    fn close_requested(&mut self, cx: &mut Context<Self>, _surface: SurfaceId) {
        self.cancel(cx);
    }

    fn pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: PointerEvent) {
        if Some(surface) == self.window && !self.done {
            self.pointer_event(cx, surface, &event);
        }
    }

    fn keyboard(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: KeyEvent) {
        if Some(surface) == self.window && !self.done {
            self.key_event(cx, &event);
        }
    }

    fn modifiers_changed(&mut self, _cx: &mut Context<Self>, _surface: SurfaceId, modifiers: Modifiers) {
        self.mods = modifiers;
    }

    fn ime(&mut self, cx: &mut Context<Self>, _surface: SurfaceId, event: ImeEvent) {
        let focus = self.focus;
        let Some(input) = self.focused_input() else { return };
        let r = match &event {
            ImeEvent::Preedit { text, cursor } => input.ime_preedit(text, *cursor),
            ImeEvent::Commit(text) => input.ime_commit(text),
            ImeEvent::DeleteSurrounding { before, after } => input.ime_delete_surrounding(*before, *after),
        };
        self.text_event(cx, focus, r);
    }

    fn paste(&mut self, cx: &mut Context<Self>, id: PasteRequestId, result: Result<nuxut::clipboard::ClipboardText, nuxut::clipboard::PasteError>) {
        let Some((_, focus)) = self.paste.take_if(|(pending, _)| *pending == id) else { return };
        let Ok(text) = result else { return };
        // Tek satırlık alan: ilk satır.
        let line = text.as_str().lines().next().unwrap_or("").to_owned();
        if focus != self.focus && self.modal == Modal::None {
            return;
        }
        let Some(input) = self.focused_input() else { return };
        let r = input.paste(&line);
        self.text_event(cx, focus, r);
    }

    fn message(&mut self, cx: &mut Context<Self>, message: Message) {
        match message {
            Message::Listed(r) => self.listed(cx, r),
            Message::Media(e) => self.media_event(cx, e),
            Message::FolderCreated(Ok(name)) => {
                self.close_modal(cx);
                self.pending_select = Some(name);
                self.load(cx);
            }
            Message::FolderCreated(Err(kind)) => {
                let text = match kind {
                    std::io::ErrorKind::AlreadyExists => t!("shell-filechooser-error-exists"),
                    std::io::ErrorKind::PermissionDenied => t!("shell-filechooser-error-denied"),
                    _ => t!("shell-filechooser-error-create"),
                };
                self.set_status(cx, Some(text));
            }
            Message::Thumb(t) => self.thumb_arrived(cx, t),
            Message::Lifeline => {
                tracing::info!("portal isteği kapattı");
                self.cancel(cx);
            }
        }
    }

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        if self.wake == Some(id) {
            self.wake = None;
            self.redraw(cx);
        }
    }

    fn theme_changed(&mut self, cx: &mut Context<Self>) {
        if self.started {
            self.build_widgets(cx);
        }
    }

    fn locale_changed(&mut self, cx: &mut Context<Self>) {
        if self.started {
            self.build_places();
            self.build_widgets(cx);
        }
    }

    fn draw(&mut self, cx: &mut Context<Self>, surface: SurfaceId, now: Instant, painter: &mut Painter) {
        if Some(surface) == self.window {
            self.draw_window(cx, surface, now, painter);
        }
    }

    fn accessibility(&mut self, cx: &mut Context<Self>, surface: SurfaceId, tree: &mut nuxut::access::AccessTree) {
        if Some(surface) == self.window {
            let theme = cx.theme().clone();
            self.access(tree, &theme);
        }
    }

    fn accessibility_action(&mut self, cx: &mut Context<Self>, surface: SurfaceId, request: nuxut::access::AccessRequest) {
        if Some(surface) == self.window {
            self.access_action(cx, surface, request);
        }
    }
}
