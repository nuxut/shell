//! Bildirim veri türleri ve UI tarafı modeli [`NotificationCenter`].
//!
//! Model saftır: thread yok, IO yok, zaman `now: Instant` ile verilir. UI sunucudan gelen
//! [`NotifyEvent`]'leri [`NotificationCenter::on_event`]'e verir, timer'ı
//! [`NotificationCenter::next_deadline`] ile kurar, süre dolunca [`NotificationCenter::expire`]
//! çağırır. Modelin kapatılmasını istediği bildirimler (geçmiş sınırı, süresi dolan transient ya da
//! geçmişi kapalı uygulama, DND'de transient) [`NotificationCenter::take_closed`] ile alınıp `NotifyHandle::close`'a
//! verilir; böylece D-Bus istemcileri `NotificationClosed` alır.
//!
//! Kurallar:
//! - Critical kendiliğinden kapanmaz (`critical_timeout_ms` verilmedikçe), istemci süre verse
//!   bile.
//! - DND, critical dışındaki popup'ları bastırır; bildirim yine geçmişe yazılır.
//! - `replaces_id` ile gelen bildirim yerinde güncellenir (aynı kimlik, en üste çıkar, popup
//!   süresi yeniden başlar, okunmamış olur).
//! - Transient geçmişe girmez: yalnızca popup'tır; popup'ı bitince kapanır.
//! - Uygulama kuralı (`[notifications.apps."<app_name>"]`): `popups = false` popup'ı (critical
//!   dahil) kapatır; `history = false` bildirimi transient gibi yalnızca popup yapar.
//! - Geçmiş sınırı gösterilmekte olan popup'ı atmaz; `history_limit = 0` iken bildirim popup'ı
//!   bitince kapanır.
//! - Popup süresi dolunca bildirim merkezde kalır (D-Bus açısından canlı, `persistence`).
//! - Fare popup üzerindeyken ([`NotificationCenter::set_hovered`]) sayaç durur.
//! - Shell'in kendi bildirimleri [`NotificationCenter::add_internal`] ile doğrudan eklenir
//!   (bkz. [`super::internal`]); sessiz olan popup üretmez, merkezde okunmamış görünür.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;

use super::hints::{NotifyIcon, Urgency};
use super::internal::{
    INTERNAL_APP_NAME, INTERNAL_ID_BASE, InternalNotice, MAX_INTERNAL_COMMAND_BYTES, NoticeMood, image_fits,
    is_internal_id,
};
use super::markup::{BodySpan, plain_body, sanitize, truncate_bytes, truncate_chars};
use super::server::{MAX_ACTION_TEXT_CHARS, MAX_ACTIONS, MAX_BODY_BYTES, MAX_SUMMARY_CHARS};
use nuxut::i18n::t;
use crate::schema::SettingSchema;

/// `NotificationClosed` sinyalinin `reason` değeri.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum CloseReason {
    Expired = 1,
    Dismissed = 2,
    ClosedByCall = 3,
    Undefined = 4,
}

/// Bildirim eylemi (düğme). `"default"` anahtarı kartın kendisine tıklamadır ve
/// [`Notification::actions`]'ta yer almaz ([`Notification::has_default_action`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyAction {
    pub key: String,
    pub label: String,
}

/// İstemcinin istediği popup süresi.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expiry {
    /// `expire_timeout == -1`: sunucu varsayılanı.
    Default,
    /// `expire_timeout == 0`.
    Never,
    After(Duration),
}

impl Expiry {
    /// D-Bus `expire_timeout` değerinden.
    pub fn from_dbus(ms: i32) -> Self {
        match ms {
            0 => Self::Never,
            t if t > 0 => Self::After(Duration::from_millis(u64::from(t.unsigned_abs()))),
            _ => Self::Default,
        }
    }
}

/// Bir bildirim. Sunucu doğrulamış ve sınırlamıştır.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub app_icon: NotifyIcon,
    pub summary: String,
    pub body: Vec<BodySpan>,
    /// Düğmeler (`default` hariç), en fazla 8.
    pub actions: Vec<NotifyAction>,
    /// Kartın kendisine tıklanınca `invoke_action(id, "default")` çağrılmalı mı.
    pub has_default_action: bool,
    pub urgency: Urgency,
    pub expire: Expiry,
    /// İçerik görseli (`image-data` / `image-path`; ör. avatar).
    pub image: NotifyIcon,
    pub category: Option<String>,
    pub desktop_entry: Option<String>,
    pub transient: bool,
    pub resident: bool,
    /// `suppress-sound` hint'i.
    pub suppress_sound: bool,
    pub received: SystemTime,
    /// Kartta maskot. YALNIZ dahili bildirimlerde ([`InternalNotice::with_mascot`]); D-Bus
    /// sunucusu hep `None` kurar ve [`NotificationCenter::on_event`] dış olayda alanı siler.
    pub mascot: Option<NoticeMood>,
}

impl Notification {
    /// Geçmişte gruplama anahtarı: masaüstü girdisi, yoksa uygulama adı (küçük harf).
    pub fn group_key(&self) -> String {
        self.desktop_entry.as_deref().unwrap_or(&self.app_name).to_lowercase()
    }
}

/// Sunucunun UI'ya verdiği olaylar.
/// Olaylar seyrek; `Added`'ı kutulamak sözleşmeyi değiştirirdi.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, PartialEq)]
pub enum NotifyEvent {
    /// Yeni bildirim ya da `replaces_id` ile değişen (aynı `id`).
    Added(Notification),
    /// Bildirim kapandı (uygulama `CloseNotification`, eylem sonrası, `NotifyHandle::close`,
    /// ya da sunucu adı kaybetti → `Undefined`). Bilinmeyen kimlik yok sayılmalı.
    Closed { id: u32, reason: CloseReason },
    /// Arka planda çözülen görsel hazır: bildirim, çözme 100 ms'yi aştığı için `Added` ile
    /// görselsiz eklenmişti. Yalnızca `Some` alanlar güncellenir; sıra, süre ve okunmamış
    /// durumu değişmez. Bilinmeyen kimlik yok sayılmalı.
    ImageReady { id: u32, app_icon: Option<NotifyIcon>, image: Option<NotifyIcon> },
}

/// Debug redakte (log politikası): bildirim metni, uygulama adı ve eylemler yazılmaz.
impl std::fmt::Debug for NotifyEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Added(n) => f.debug_struct("Added").field("id", &n.id).finish_non_exhaustive(),
            Self::Closed { id, reason } => f.debug_struct("Closed").field("id", id).field("reason", reason).finish(),
            Self::ImageReady { id, app_icon, image } => f
                .debug_struct("ImageReady")
                .field("id", id)
                .field("app_icon", &app_icon.is_some())
                .field("image", &image.is_some())
                .finish(),
        }
    }
}

// ---------------------------------------------------------------------------------------
// Ayarlar

/// `shell.toml` `[notifications]` (şema Ayarlar uygulamasıyla ortak).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct NotificationsConfig {
    /// `false`: sunucu hiç başlatılmaz (shell tarafında uygulanır).
    pub enabled: bool,
    /// Rahatsız etme kipi.
    pub dnd: bool,
    /// Low/normal ve `expire_timeout == -1` için popup süresi (1000..=60000).
    pub popup_timeout_ms: u64,
    /// Critical popup süresi; `None` = kapanmaz.
    pub critical_timeout_ms: Option<u64>,
    /// 1..=10.
    pub max_popups: usize,
    /// 0..=500; 0 = geçmiş tutulmaz (popup bitince bildirim kapanır).
    pub history_limit: usize,
    /// Uygulama başına kurallar: `[notifications.apps."<app_name>"]`. Anahtar `app_name` ile
    /// birebir, büyük/küçük harf duyarlı eşleşir.
    pub apps: BTreeMap<String, AppRule>,
}

impl Default for NotificationsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dnd: false,
            popup_timeout_ms: 5000,
            critical_timeout_ms: None,
            max_popups: 4,
            history_limit: 100,
            apps: BTreeMap::new(),
        }
    }
}

/// Bir uygulamanın bildirim kuralı.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct AppRule {
    /// `false`: popup çıkmaz (critical dahil), bildirim yalnızca merkeze düşer.
    pub popups: bool,
    /// `false`: geçmişe girmez; popup bitince kapanır.
    pub history: bool,
}

impl Default for AppRule {
    fn default() -> Self {
        Self { popups: true, history: true }
    }
}

/// En fazla uygulama kuralı.
pub const MAX_APP_RULES: usize = 256;
/// Kural anahtarının (uygulama adı) en uzun hali (karakter).
pub const MAX_APP_RULE_NAME_CHARS: usize = 128;

/// İstemcinin isteyebileceği en uzun popup süresi.
pub const MAX_CLIENT_TIMEOUT: Duration = Duration::from_secs(3600);
/// Fareden çıkınca kalan süre en az bu kadar olur.
pub const RESUME_GRACE: Duration = Duration::from_secs(1);

/// Popup süresi sınırları (ms); critical süre de aynı alt sınırı kullanır.
pub const MIN_POPUP_TIMEOUT_MS: u64 = 1000;
pub const MAX_POPUP_TIMEOUT_MS: u64 = 60_000;
pub const MAX_CRITICAL_TIMEOUT_MS: u64 = 3_600_000;
pub const MAX_POPUP_COUNT: usize = 10;
pub const MAX_HISTORY_LIMIT: usize = 500;

impl NotificationsConfig {
    /// `[notifications]` anahtarlarının şeması (`nuxut-shell schema`). `critical_timeout_ms`
    /// (yokluğu anlamlı) ve `apps` (serbest tablo) şemada yok.
    pub fn schema(out: &mut Vec<SettingSchema>) {
        let d = Self::default();
        out.push(SettingSchema::bool("notifications.enabled", t!("shell-notifications-title"), d.enabled));
        out.push(SettingSchema::bool("notifications.dnd", t!("shell-notifications-dnd-label"), d.dnd));
        out.push(
            SettingSchema::int(
                "notifications.popup_timeout_ms",
                t!("shell-notifications-popup-timeout-label"),
                d.popup_timeout_ms as i64,
                MIN_POPUP_TIMEOUT_MS as i64,
                MAX_POPUP_TIMEOUT_MS as i64,
            )
            .step(500.0)
            .unit("ms"),
        );
        out.push(SettingSchema::int("notifications.max_popups", t!("shell-notifications-max-popups-label"), d.max_popups as i64, 1, MAX_POPUP_COUNT as i64));
        out.push(SettingSchema::int(
            "notifications.history_limit",
            t!("shell-notifications-history-limit-label"),
            d.history_limit as i64,
            0,
            MAX_HISTORY_LIMIT as i64,
        ));
    }

    /// Değerleri sınırlar; her düzeltme için bir uyarı döndürür.
    pub fn validate(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();
        let mut clamp = |name: &str, value: &mut u64, min: u64, max: u64| {
            let clamped = (*value).clamp(min, max);
            if clamped != *value {
                warnings
                    .push(format!("notifications.{name} = {value} geçersiz ({min}..={max}); {clamped} kullanılıyor"));
                *value = clamped;
            }
        };
        clamp("popup_timeout_ms", &mut self.popup_timeout_ms, MIN_POPUP_TIMEOUT_MS, MAX_POPUP_TIMEOUT_MS);
        if let Some(ms) = self.critical_timeout_ms.as_mut() {
            clamp("critical_timeout_ms", ms, MIN_POPUP_TIMEOUT_MS, MAX_CRITICAL_TIMEOUT_MS);
        }
        let mut max_popups = self.max_popups as u64;
        clamp("max_popups", &mut max_popups, 1, MAX_POPUP_COUNT as u64);
        self.max_popups = max_popups as usize;
        let mut history = self.history_limit as u64;
        clamp("history_limit", &mut history, 0, MAX_HISTORY_LIMIT as u64);
        self.history_limit = history as usize;

        self.apps.retain(|name, _| {
            let ok = !name.is_empty() && name.chars().count() <= MAX_APP_RULE_NAME_CHARS;
            if !ok {
                warnings.push(format!(
                    "notifications.apps: geçersiz uygulama adı (boş ya da {MAX_APP_RULE_NAME_CHARS} karakterden uzun) atıldı"
                ));
            }
            ok
        });
        if self.apps.len() > MAX_APP_RULES {
            let dropped = self.apps.len() - MAX_APP_RULES;
            while self.apps.len() > MAX_APP_RULES {
                self.apps.pop_last();
            }
            warnings.push(format!("notifications.apps: en fazla {MAX_APP_RULES} kural; {dropped} kural atıldı"));
        }
        warnings
    }

    /// Uygulamanın kuralı (yoksa varsayılan: popup ve geçmiş açık).
    pub fn rule(&self, app_name: &str) -> AppRule {
        self.apps.get(app_name).copied().unwrap_or_default()
    }

    /// Popup'ın kalacağı süre; `None` = kalıcı.
    pub fn popup_duration(&self, n: &Notification) -> Option<Duration> {
        if n.urgency == Urgency::Critical {
            return self.critical_timeout_ms.map(Duration::from_millis);
        }
        match n.expire {
            Expiry::Default => Some(Duration::from_millis(self.popup_timeout_ms)),
            Expiry::Never => None,
            Expiry::After(d) => Some(d.min(MAX_CLIENT_TIMEOUT)),
        }
    }
}

// ---------------------------------------------------------------------------------------
// Model

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Timer {
    Running { deadline: Instant },
    Paused { remaining: Duration },
    Persistent,
}

#[derive(Debug, Clone)]
struct Popup {
    timer: Timer,
    hovered: bool,
}

#[derive(Debug, Clone)]
struct Entry {
    notification: Notification,
    popup: Option<Popup>,
    read: bool,
    /// Merkezde (geçmişte) görünür mü: transient değil ve uygulama kuralı izin veriyor.
    in_history: bool,
}

/// Geçmişte bir uygulamanın bildirimleri (en yenisi önce).
#[derive(Debug, Clone, PartialEq)]
pub struct HistoryGroup<'a> {
    pub key: String,
    pub app_name: &'a str,
    pub desktop_entry: Option<&'a str>,
    pub notifications: Vec<&'a Notification>,
}

/// Popup'lar ve geçmiş (bildirim merkezi). Bkz. modül belgesi.
#[derive(Debug, Clone)]
pub struct NotificationCenter {
    config: NotificationsConfig,
    dnd: bool,
    /// En yenisi başta. Transient girdiler yalnızca popup iken burada.
    entries: VecDeque<Entry>,
    /// Modelin kapattığı, sunucuya bildirilecek kimlikler.
    closed: Vec<(u32, CloseReason)>,
    /// Dahili bildirimlerin eylem komutları: kimlik → (eylem anahtarı, komut satırı).
    internal: BTreeMap<u32, Vec<(String, String)>>,
    /// Son verilen dahili kimlik.
    last_internal: u32,
}

impl Default for NotificationCenter {
    fn default() -> Self {
        Self::new(NotificationsConfig::default())
    }
}

impl NotificationCenter {
    pub fn new(config: NotificationsConfig) -> Self {
        let dnd = config.dnd;
        Self {
            config,
            dnd,
            entries: VecDeque::new(),
            closed: Vec::new(),
            internal: BTreeMap::new(),
            last_internal: INTERNAL_ID_BASE - 1,
        }
    }

    /// Shell'in kendi bildirimini ekler (D-Bus'sız, bkz. [`super::internal`]); kimliğini döner.
    /// Metinler sunucu sınırlarıyla kırpılır. Sessiz bildirim (`popup = false`) popup üretmez,
    /// merkezde okunmamış görünür; `history_limit = 0` iken hiç görünmez.
    pub fn add_internal(&mut self, notice: InternalNotice, now: Instant) -> u32 {
        // Artık modelde olmayan dahili bildirimlerin eylemleri unutulur (harita geçmişle sınırlı kalır).
        let live: Vec<u32> = self.internal.keys().copied().filter(|&id| self.find(id).is_some()).collect();
        self.internal.retain(|id, _| live.contains(id));
        let id = self.alloc_internal_id();

        let one_line = |s: &str, max| sanitize(truncate_chars(s.trim(), max)).replace(['\n', '\t'], " ");
        let mut actions = Vec::new();
        let mut commands = Vec::new();
        for (i, a) in notice.actions.iter().enumerate() {
            let command = a.command.trim();
            if actions.len() >= MAX_ACTIONS || command.is_empty() || command.len() > MAX_INTERNAL_COMMAND_BYTES {
                continue;
            }
            let key = format!("nuxut-internal-{i}");
            actions.push(NotifyAction { key: key.clone(), label: one_line(&a.label, MAX_ACTION_TEXT_CHARS) });
            commands.push((key, command.to_owned()));
        }
        let image = if notice.mascot.is_some() {
            // Tek görsel: maskotlu kartta içerik görseli yok (maskot framework kartının sol yuvasında).
            NotifyIcon::None
        } else if image_fits(&notice.image) {
            notice.image
        } else {
            tracing::warn!(?notice.image, "dahili bildirim görseli sınır dışı; atıldı");
            NotifyIcon::None
        };
        let n = Notification {
            id,
            app_name: INTERNAL_APP_NAME.to_owned(),
            app_icon: NotifyIcon::None,
            summary: one_line(&notice.summary, MAX_SUMMARY_CHARS),
            body: plain_body(truncate_bytes(notice.body.trim(), MAX_BODY_BYTES)),
            actions,
            has_default_action: false,
            urgency: Urgency::Normal,
            expire: Expiry::Default,
            image,
            category: None,
            desktop_entry: None,
            transient: false,
            resident: false,
            suppress_sound: !notice.popup,
            received: SystemTime::now(),
            mascot: notice.mascot,
        };
        if !commands.is_empty() {
            self.internal.insert(id, commands);
        }
        self.insert(n, now, notice.popup);
        id
    }

    fn alloc_internal_id(&mut self) -> u32 {
        loop {
            self.last_internal = match self.last_internal.checked_add(1) {
                Some(id) => id,
                None => INTERNAL_ID_BASE,
            };
            if self.find(self.last_internal).is_none() {
                return self.last_internal;
            }
        }
    }

    /// Dahili bildirim eyleminin komut satırı (`nuxut-shell msg` sözdizimi). Bildirim modelde
    /// değilse ya da eylem yoksa `None`; D-Bus bildirimleri için her zaman `None`.
    pub fn internal_command(&self, id: u32, key: &str) -> Option<&str> {
        if !is_internal_id(id) || self.find(id).is_none() {
            return None;
        }
        self.internal.get(&id)?.iter().find(|(k, _)| k == key).map(|(_, c)| c.as_str())
    }

    pub fn config(&self) -> &NotificationsConfig {
        &self.config
    }

    /// Ayarları değiştirir (canlı `shell.toml`); sınırlar hemen uygulanır.
    pub fn set_config(&mut self, config: NotificationsConfig) {
        self.dnd = config.dnd;
        self.config = config;
        if self.dnd {
            self.suppress_popups();
        }
        self.enforce_limits();
    }

    pub fn dnd(&self) -> bool {
        self.dnd
    }

    pub fn get(&self, id: u32) -> Option<&Notification> {
        self.find(id).map(|i| &self.entries[i].notification)
    }

    fn find(&self, id: u32) -> Option<usize> {
        self.entries.iter().position(|e| e.notification.id == id)
    }

    /// Sunucu olayını uygular. Görünür bir şey değiştiyse `true`.
    pub fn on_event(&mut self, event: NotifyEvent, now: Instant) -> bool {
        match event {
            NotifyEvent::Added(mut n) => {
                // Dış (D-Bus) bildirim maskot taşıyamaz: sunucu zaten `None` kurar; model de
                // güvenmez (maskot yalnızca `add_internal` yolundan girer).
                n.mascot = None;
                self.add(n, now);
                true
            }
            NotifyEvent::Closed { id, .. } => match self.find(id) {
                Some(i) => {
                    self.entries.remove(i);
                    true
                }
                None => false,
            },
            NotifyEvent::ImageReady { id, app_icon, image } => {
                let Some(i) = self.find(id) else { return false };
                let n = &mut self.entries[i].notification;
                let mut changed = false;
                if let Some(icon) = app_icon {
                    n.app_icon = icon;
                    changed = true;
                }
                if let Some(icon) = image {
                    n.image = icon;
                    changed = true;
                }
                changed
            }
        }
    }

    /// Modeldeki görsellerin toplam piksel baytı (bellek ölçümü için).
    pub fn image_bytes(&self) -> usize {
        self.entries.iter().map(|e| e.notification.app_icon.byte_size() + e.notification.image.byte_size()).sum()
    }

    fn add(&mut self, n: Notification, now: Instant) {
        self.insert(n, now, true);
    }

    /// `allow_popup = false`: kurallar ne derse desin popup yok (sessiz dahili bildirim).
    fn insert(&mut self, n: Notification, now: Instant, allow_popup: bool) {
        let id = n.id;
        let old = self.find(id).and_then(|i| self.entries.remove(i));
        let rule = self.config.rule(&n.app_name);
        let in_history = !n.transient && rule.history;
        let show = allow_popup && rule.popups && (!self.dnd || n.urgency == Urgency::Critical);
        if !show && !in_history {
            // Ne popup ne geçmiş (DND'de transient, ya da uygulama kuralı ikisini de kapatmış).
            self.closed.push((id, CloseReason::Expired));
            return;
        }
        let popup = show.then(|| {
            let hovered = old.as_ref().and_then(|e| e.popup.as_ref()).is_some_and(|p| p.hovered);
            let timer = match (self.config.popup_duration(&n), hovered) {
                (None, _) => Timer::Persistent,
                (Some(remaining), true) => Timer::Paused { remaining },
                (Some(d), false) => Timer::Running { deadline: now + d },
            };
            Popup { timer, hovered }
        });
        self.entries.push_front(Entry { notification: n, popup, read: false, in_history });
        self.enforce_limits();
    }

    fn enforce_limits(&mut self) {
        // Geçmiş sınırı merkezdekileri sayar; gösterilmekte olan popup asla atılmaz (sınır 0
        // iken bile popup süresini tamamlar, sonra kapanır).
        while self.history().count() > self.config.history_limit {
            let victim = self.entries.iter().rposition(|e| e.in_history && e.popup.is_none());
            let Some(i) = victim else { break };
            if let Some(e) = self.entries.remove(i) {
                self.closed.push((e.notification.id, CloseReason::Expired));
            }
        }
        let max = self.config.max_popups.max(1);
        while self.entries.iter().filter(|e| e.popup.is_some()).count() > max {
            let victim = self
                .entries
                .iter()
                .rposition(|e| e.popup.is_some() && e.notification.urgency != Urgency::Critical)
                .or_else(|| self.entries.iter().rposition(|e| e.popup.is_some()));
            let Some(i) = victim else { break };
            self.end_popup(i, CloseReason::Expired);
        }
    }

    /// `i`'nin popup'ını bitirir; transient ise bildirim kapanır. Girdi silindiyse `true`.
    fn end_popup(&mut self, i: usize, reason: CloseReason) -> bool {
        if !self.entries[i].in_history {
            if let Some(e) = self.entries.remove(i) {
                self.closed.push((e.notification.id, reason));
            }
            true
        } else {
            self.entries[i].popup = None;
            false
        }
    }

    /// Görünür popup'lar, en yenisi önce (≤ `max_popups`).
    pub fn popups(&self) -> impl Iterator<Item = &Notification> {
        self.entries.iter().filter(|e| e.popup.is_some()).map(|e| &e.notification)
    }

    pub fn is_popup(&self, id: u32) -> bool {
        self.find(id).is_some_and(|i| self.entries[i].popup.is_some())
    }

    /// Geçmiş (merkez), en yenisi önce (≤ `history_limit`); transient hariç.
    pub fn history(&self) -> impl Iterator<Item = &Notification> {
        self.entries.iter().filter(|e| e.in_history).map(|e| &e.notification)
    }

    /// Geçmiş uygulamaya göre gruplanmış; gruplar en yeni bildirimlerine göre sıralı.
    pub fn history_groups(&self) -> Vec<HistoryGroup<'_>> {
        let mut groups: Vec<HistoryGroup<'_>> = Vec::new();
        for n in self.history() {
            let key = n.group_key();
            match groups.iter_mut().find(|g| g.key == key) {
                Some(g) => g.notifications.push(n),
                None => groups.push(HistoryGroup {
                    key,
                    app_name: &n.app_name,
                    desktop_entry: n.desktop_entry.as_deref(),
                    notifications: vec![n],
                }),
            }
        }
        groups
    }

    pub fn unread_count(&self) -> usize {
        self.entries.iter().filter(|e| e.in_history && !e.read).count()
    }

    pub fn mark_all_read(&mut self) {
        for e in &mut self.entries {
            e.read = true;
        }
    }

    /// Popup'ı kaldırır, bildirim merkezde kalır (transient ise kapanır → `take_closed`).
    pub fn dismiss_popup(&mut self, id: u32) -> bool {
        match self.find(id).filter(|&i| self.entries[i].popup.is_some()) {
            Some(i) => {
                self.end_popup(i, CloseReason::Dismissed);
                self.enforce_limits();
                true
            }
            None => false,
        }
    }

    /// Kullanıcı kapattı: bildirimi siler, sunucuya `close(id, Dismissed)` için kimliği döner.
    pub fn remove(&mut self, id: u32) -> Option<u32> {
        let i = self.find(id)?;
        self.entries.remove(i);
        Some(id)
    }

    /// Merkezi temizler (popup'lar dahil); kapatılacak kimlikler.
    pub fn clear_all(&mut self) -> Vec<u32> {
        self.entries.drain(..).map(|e| e.notification.id).collect()
    }

    /// Rahatsız etme kipi; açılınca critical olmayan popup'lar gizlenir.
    pub fn set_dnd(&mut self, on: bool) {
        self.dnd = on;
        if on {
            self.suppress_popups();
            self.enforce_limits();
        }
    }

    fn suppress_popups(&mut self) {
        let mut i = 0;
        while i < self.entries.len() {
            let e = &self.entries[i];
            if e.popup.is_some()
                && e.notification.urgency != Urgency::Critical
                && self.end_popup(i, CloseReason::Dismissed)
            {
                continue;
            }
            i += 1;
        }
    }

    /// Fare popup'ın üzerinde mi: üstündeyken sayaç durur, ayrılınca kalan süreyle (en az
    /// [`RESUME_GRACE`]) devam eder. Tek işaretçi aynı anda tek popup'ın üzerinde olabilir:
    /// `hovered = true` diğer üzerinde sayılan popup'ları bırakır (karttan karta geçişte tek
    /// olay yeter). Değişiklik olduysa `true` (timer'ı yeniden kur).
    pub fn set_hovered(&mut self, id: u32, hovered: bool, now: Instant) -> bool {
        if self.find(id).and_then(|i| self.entries[i].popup.as_ref()).is_none() {
            return false;
        }
        let mut changed = false;
        for e in &mut self.entries {
            let target = hovered && e.notification.id == id;
            if let Some(popup) = e.popup.as_mut()
                && (e.notification.id == id || hovered)
                && popup.hovered != target
            {
                Self::apply_hover(popup, target, now);
                changed = true;
            }
        }
        changed
    }

    fn apply_hover(popup: &mut Popup, hovered: bool, now: Instant) {
        popup.hovered = hovered;
        popup.timer = match (popup.timer, hovered) {
            (Timer::Running { deadline }, true) => Timer::Paused { remaining: deadline.saturating_duration_since(now) },
            (Timer::Paused { remaining }, false) => Timer::Running { deadline: now + remaining.max(RESUME_GRACE) },
            (timer, _) => timer,
        };
    }

    /// En yakın popup süre dolumu (timer bununla kurulur).
    pub fn next_deadline(&self) -> Option<Instant> {
        self.entries
            .iter()
            .filter_map(|e| match e.popup {
                Some(Popup { timer: Timer::Running { deadline }, .. }) => Some(deadline),
                _ => None,
            })
            .min()
    }

    /// Süresi dolan popup'ları bitirir ve kimliklerini döndürür. Transient olanlar ayrıca
    /// kapanır (`take_closed`).
    pub fn expire(&mut self, now: Instant) -> Vec<u32> {
        let mut expired = Vec::new();
        let mut i = 0;
        while i < self.entries.len() {
            let due = matches!(
                self.entries[i].popup,
                Some(Popup { timer: Timer::Running { deadline }, .. }) if deadline <= now
            );
            if due {
                expired.push(self.entries[i].notification.id);
                if self.end_popup(i, CloseReason::Expired) {
                    continue;
                }
            }
            i += 1;
        }
        self.enforce_limits();
        expired
    }

    /// Modelin kendiliğinden kapattığı bildirimler (geçmiş sınırı, transient). UI bunları
    /// `NotifyHandle::close(id, reason)`'a verir.
    pub fn take_closed(&mut self) -> Vec<(u32, CloseReason)> {
        // Dahili bildirimlerin D-Bus karşılığı yok.
        let mut closed = std::mem::take(&mut self.closed);
        closed.retain(|&(id, _)| !is_internal_id(id));
        closed
    }
}
