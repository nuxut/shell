//! Ekran yöneticisi (ADR-0005 §4, §5, §8): `list/status/apply/confirm/revert`, onay zamanlayıcısı,
//! hotplug küme hafızası, açılışta yarım kalan değişiklikten geri dönüş.
//!
//! Durum makinesi:
//! ```text
//! Idle --apply (onaylı)--> Pending{jeton, önceki, istenen, son an}
//! Idle --apply (yalnızca konum/birincil)--> uygula + kalıcı yaz --> Idle
//! Pending --confirm(jeton)--> kalıcı yaz, bekçiyi kaldır --> Idle
//! Pending --revert(jeton) | süre doldu | hotplug--> önceki kuralları uygula --> Idle
//! Pending --apply--> error: pending_exists
//! ```
//! Hyprland'e okuma için `reader`, yazma için yalnızca `writer` (kullanıcının oturumu ya da
//! yalıtılmış test Hyprland'i; aksi hâlde `None` → `apply` = `unavailable`). Zaman çağırandan
//! gelir ([`Now`]); testlerde sahte saat.

use std::time::{Duration, Instant};

use super::guard::{PendingRecord, valid_token};
use super::model::{DisplayLayout, Hardware};
use super::protocol::{ApiError, LastJson, PendingJson, StatusJson, apply_json, list_json, parse_apply, provider_name};
use super::render::{Guard, fallback_command, file_text, legacy_guard_exec, lua_disarm, runtime_command};
use super::store::Store;
use super::validate::{Issue, validate};
use crate::services::hypr_config::control::HyprControl;
use crate::services::hypr_config::sync::ConfigKind;

/// Onay süresi sınırları (sn; `shell.toml [displays] confirm_seconds`).
pub const MIN_CONFIRM_SECONDS: u32 = 5;
pub const MAX_CONFIRM_SECONDS: u32 = 60;
pub const DEFAULT_CONFIRM_SECONDS: u32 = 15;
/// Hyprland tarafındaki ölü adam anahtarının shell zamanlayıcısına payı.
pub const GUARD_MARGIN_MS: u64 = 5_000;

/// Dosya erişimi (gerçek: `app/displays.rs`; testte bellek).
pub trait DisplayFiles {
    /// `displays.toml` metni (yoksa `None`).
    fn read_store(&self) -> Option<String>;
    fn write_store(&self, text: &str) -> Result<(), String>;
    /// Üretilen Hyprland dosyası. `create = false`: dosya yoksa oluşturma (yalnızca güncelle).
    fn write_generated(&self, kind: ConfigKind, text: &str, create: bool) -> Result<(), String>;
    fn read_pending(&self) -> Option<PendingRecord>;
    fn write_pending(&self, record: &PendingRecord) -> Result<(), String>;
    fn remove_pending(&self);
}

/// Şimdiki an: tekdüze (shell zamanlayıcısı) + Unix ms (bekçi ve kayıt).
#[derive(Debug, Clone, Copy)]
pub struct Now {
    pub mono: Instant,
    pub unix_ms: u64,
}

impl Now {
    pub fn system() -> Self {
        Self {
            mono: Instant::now(),
            unix_ms: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub confirm_seconds: u32,
    /// IPC uygulama adı (bekçi ve kayıt dosyası).
    pub app: String,
    /// Kurulu ikili (hyprlang bekçisi; güvensizse bekçi başlatılmaz, shell zamanlayıcısı kalır).
    pub exe: String,
    /// Uyguladıktan sonra sonucu doğrulama denemeleri ve aralığı.
    pub verify_tries: u32,
    pub verify_interval: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            confirm_seconds: DEFAULT_CONFIRM_SECONDS,
            app: crate::ipc::APP_NAME.into(),
            exe: String::new(),
            verify_tries: 6,
            verify_interval: Duration::from_millis(150),
        }
    }
}

/// `apply` başarılı cevabı.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub token: String,
    /// Onay bekleniyor (değilse değişiklik kalıcı yazıldı).
    pub confirm: bool,
    pub deadline_ms: u64,
}

impl Applied {
    pub fn json(&self) -> String {
        serde_json::json!({"token": self.token, "confirm": self.confirm, "deadline_ms": self.deadline_ms}).to_string()
    }
}

#[derive(Debug, Clone)]
struct Pending {
    token: String,
    previous: DisplayLayout,
    proposed: DisplayLayout,
    set_key: Vec<String>,
    kind: ConfigKind,
    deadline: Instant,
}

/// Ekran yöneticisi; tek thread'de yaşar (`app/displays.rs`).
pub struct DisplayManager {
    opts: Options,
    reader: Box<dyn HyprControl>,
    writer: Option<Box<dyn HyprControl>>,
    files: Box<dyn DisplayFiles>,
    store: Store,
    hw: Option<Hardware>,
    provider: Option<ConfigKind>,
    r#gen: u64,
    pending: Option<Pending>,
    last: Option<LastJson>,
    seq: u64,
    /// Hotplug işlemesinin son gördüğü bağlı küme. `refresh` (list/apply/doğrulama) kümeyi
    /// güncellese de hotplug kararı bununla karşılaştırılır; değişiklik "tüketilmez".
    handled_set: Option<Vec<String>>,
}

impl std::fmt::Debug for DisplayManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DisplayManager")
            .field("gen", &self.r#gen)
            .field("provider", &self.provider)
            .field("writable", &self.writer.is_some())
            .field("pending", &self.pending.as_ref().map(|p| &p.token))
            .finish_non_exhaustive()
    }
}

fn issue_error(issues: &[Issue]) -> ApiError {
    let Some(first) = issues.first() else { return ApiError::new("bad_json", "") };
    let mut message = match &first.display {
        Some(d) => format!("{d}: {}", first.message),
        None => first.message.clone(),
    };
    if issues.len() > 1 {
        message.push_str(&format!(" (+{} sorun daha)", issues.len() - 1));
    }
    ApiError::new(first.code.as_str(), message)
}

/// Hyprland'in cevabı başarı mı? (`eval` → `ok`; `[[BATCH]]` → her komut için `ok`.)
fn reply_ok(reply: &str) -> bool {
    let t = reply.trim();
    !t.is_empty() && t.replace("ok", "").trim().is_empty()
}

fn clip(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(200).collect()
}

/// Değişiklik onay ister mi? Yalnızca konum ve birincil değişimi onaysızdır (K2).
pub fn needs_confirm(previous: &DisplayLayout, proposed: &DisplayLayout) -> bool {
    proposed.displays.iter().any(|p| match previous.get(&p.key) {
        None => true,
        Some(o) => {
            o.enabled != p.enabled
                || (p.enabled
                    && (!o.mode.matches(&p.mode)
                        || o.scale_120 != p.scale_120
                        || o.transform != p.transform
                        || o.vrr != p.vrr
                        || o.mirror_of != p.mirror_of))
        }
    })
}

/// Hyprland'in şimdiki durumu istenenle uyuşuyor mu? Uyuşmayan monitörün anahtarı.
pub fn mismatch(want: &DisplayLayout, hw: &Hardware) -> Option<String> {
    for d in &want.displays {
        let Some(m) = hw.by_key(&d.key) else { return Some(d.key.clone()) };
        let ok = if !d.enabled {
            !m.enabled
        } else {
            let target = d.mirror_of.as_deref().and_then(|k| hw.by_key(k)).map(|t| t.connector.as_str());
            m.enabled
                && m.mode.matches(&d.mode)
                && m.scale_120 == d.scale_120
                && m.transform == d.transform
                && m.mirror_of.as_deref() == target
                && (d.mirror_of.is_some() || m.pos == d.pos)
        };
        if !ok {
            return Some(d.key.clone());
        }
    }
    None
}

impl DisplayManager {
    pub fn new(opts: Options, reader: Box<dyn HyprControl>, writer: Option<Box<dyn HyprControl>>, files: Box<dyn DisplayFiles>) -> Self {
        let (store, warnings) = files.read_store().map(|t| Store::parse(&t)).unwrap_or_default();
        for w in warnings {
            tracing::warn!("ekranlar: {w}");
        }
        let confirm_seconds = opts.confirm_seconds.clamp(MIN_CONFIRM_SECONDS, MAX_CONFIRM_SECONDS);
        Self {
            opts: Options { confirm_seconds, ..opts },
            reader,
            writer,
            files,
            store,
            hw: None,
            provider: None,
            r#gen: 0,
            pending: None,
            last: None,
            seq: 0,
            handled_set: None,
        }
    }

    pub fn set_confirm_seconds(&mut self, s: u32) {
        self.opts.confirm_seconds = s.clamp(MIN_CONFIRM_SECONDS, MAX_CONFIRM_SECONDS);
    }

    pub fn r#gen(&self) -> u64 {
        self.r#gen
    }

    /// Bekleyen değişikliğin son anı (thread zamanlayıcısı için).
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(|p| p.deadline)
    }

    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    fn read_provider(&self) -> Option<ConfigKind> {
        #[derive(serde::Deserialize)]
        struct Status {
            #[serde(rename = "configProvider")]
            config_provider: Option<String>,
        }
        let text = self.reader.request("j/status").ok()?;
        match serde_json::from_str::<Status>(&text).ok()?.config_provider.as_deref() {
            Some("lua") => Some(ConfigKind::Lua),
            Some("hyprlang") => Some(ConfigKind::Hyprlang),
            _ => None,
        }
    }

    fn read_hw(&self) -> Result<Hardware, String> {
        let text = self.reader.request("j/monitors all")?;
        Hardware::parse(&text)
    }

    /// Hyprland'den yeniden okur; değiştiyse `gen` artar. Bağlı küme değişimi burada
    /// işlenmez ([`Self::on_monitors_changed`] `handled_set` ile karşılaştırır).
    pub fn refresh(&mut self) -> Result<(), String> {
        let hw = self.read_hw()?;
        if self.provider.is_none() {
            self.provider = self.read_provider();
        }
        if self.hw.as_ref() != Some(&hw) {
            self.r#gen += 1;
            self.hw = Some(hw);
        }
        Ok(())
    }

    /// Bu küme için istenen (kayıtlı) yerleşim; yoksa çalışma anı durumu (K8: içe aktarma
    /// yalnızca bellekte; ilk onaya kadar diske yazılmaz).
    fn desired(&self, hw: &Hardware) -> DisplayLayout {
        if let Some(p) = &self.pending {
            return p.proposed.clone();
        }
        match self.store.get(&hw.set_key()) {
            Some(r) => r.layout.clone(),
            None => hw.current_layout(),
        }
    }

    fn pending_json(&self, now: Instant) -> Option<PendingJson> {
        self.pending.as_ref().map(|p| PendingJson {
            token: p.token.clone(),
            remaining_ms: p.deadline.saturating_duration_since(now).as_millis() as u64,
            confirm: true,
        })
    }

    pub fn list(&mut self, now: Instant) -> Result<String, ApiError> {
        self.refresh().map_err(|e| ApiError::new("unavailable", clip(&e)))?;
        let Some(hw) = self.hw.clone() else { return Err(ApiError::new("unavailable", "")) };
        let desired = self.desired(&hw);
        Ok(list_json(&hw, Some(&desired), self.r#gen, self.provider, self.writer.is_some(), self.pending_json(now)))
    }

    pub fn status(&self, now: Instant) -> String {
        let s = StatusJson {
            v: super::protocol::VERSION,
            r#gen: self.r#gen,
            provider: provider_name(self.provider),
            available: self.writer.is_some(),
            pending: self.pending_json(now),
            last: self.last.as_ref(),
        };
        serde_json::to_string(&s).unwrap_or_default()
    }

    fn next_token(&mut self, now: Now) -> String {
        self.seq += 1;
        format!("{:08x}{:08x}", (now.unix_ms & 0xffff_ffff) as u32, (self.seq as u32) ^ std::process::id())
    }

    fn set_last(&mut self, token: &str, result: &'static str, detail: Option<String>) {
        self.last = Some(LastJson { token: token.to_owned(), result, detail });
    }

    /// Kuralları Hyprland'e gönderir (yazma erişimi zorunlu).
    fn send(&self, command: &str) -> Result<(), String> {
        let w = self.writer.as_ref().ok_or("Hyprland'e yazma erişimi yok")?;
        let reply = w.request(command)?;
        if reply_ok(&reply) { Ok(()) } else { Err(clip(&reply)) }
    }

    /// Uygulanan durumun Hyprland'de oturmasını bekler; uyuşmazlıkta anahtar.
    fn verify(&mut self, want: &DisplayLayout) -> Result<(), String> {
        let mut last = String::new();
        for i in 0..self.opts.verify_tries.max(1) {
            if i > 0 {
                std::thread::sleep(self.opts.verify_interval);
            }
            let _ = self.refresh();
            match self.hw.as_ref().map(|hw| mismatch(want, hw)) {
                Some(None) => return Ok(()),
                Some(Some(k)) => last = k,
                None => {}
            }
        }
        Err(last)
    }

    pub fn apply(&mut self, json: &str, now: Now) -> Result<Applied, ApiError> {
        if self.writer.is_none() {
            return Err(ApiError::new("unavailable", "bu örnek Hyprland'e ekran komutu göndermez"));
        }
        if self.pending.is_some() {
            return Err(ApiError::new("pending_exists", "önceki değişiklik onay bekliyor"));
        }
        let req = parse_apply(json)?;
        self.refresh().map_err(|e| ApiError::new("unavailable", clip(&e)))?;
        if req.base_gen != self.r#gen {
            return Err(ApiError::new("stale", format!("güncel gen {}", self.r#gen)));
        }
        let hw = self.hw.clone().ok_or_else(|| ApiError::new("unavailable", ""))?;
        let kind = self.provider.ok_or_else(|| ApiError::new("unavailable", "Hyprland ayar dili okunamadı"))?;
        let proposed = validate(&req.layout, &hw).map_err(|i| issue_error(&i))?;
        let previous = self.desired(&hw);
        let set_key = hw.set_key();
        let token = self.next_token(now);

        if !needs_confirm(&previous, &proposed) {
            let cmd = runtime_command(&proposed, &hw, kind, None).ok_or_else(|| ApiError::new("bad_json", "güvensiz değer"))?;
            self.send(&cmd).map_err(|e| ApiError::new("not_applied", e))?;
            if let Err(k) = self.verify(&proposed) {
                self.restore(&previous, kind);
                self.set_last(&token, "not_applied", Some(k.clone()));
                return Err(ApiError::new("not_applied", format!("{k}: Hyprland istenen durumu uygulamadı; geri dönüldü")));
            }
            self.persist(&set_key, &proposed, kind);
            self.set_last(&token, "applied", None);
            return Ok(Applied { token, confirm: false, deadline_ms: 0 });
        }

        let confirm_ms = u64::from(self.opts.confirm_seconds) * 1000;
        // Önce kayıt (shell çökerse açılışta geri dönülür), sonra bekçi + değişiklik.
        let record = PendingRecord {
            v: 1,
            token: token.clone(),
            deadline_unix_ms: now.unix_ms + confirm_ms + GUARD_MARGIN_MS,
            provider: provider_name(Some(kind)).unwrap_or_default().to_owned(),
            previous: apply_json(0, &previous),
        };
        if let Err(e) = self.files.write_pending(&record) {
            tracing::warn!("ekranlar: bekleyen değişiklik kaydı yazılamadı: {e}");
        }
        let guard = (kind == ConfigKind::Lua).then_some(Guard { previous: &previous, timeout_ms: confirm_ms + GUARD_MARGIN_MS });
        let Some(cmd) = runtime_command(&proposed, &hw, kind, guard) else {
            self.files.remove_pending();
            return Err(ApiError::new("bad_json", "güvensiz değer"));
        };
        if kind == ConfigKind::Hyprlang {
            match legacy_guard_exec(&self.opts.exe, &token, record.deadline_unix_ms, &self.opts.app) {
                Some(exec) => {
                    if let Err(e) = self.send(&exec) {
                        tracing::warn!("ekranlar: bekçi başlatılamadı (yalnızca shell zamanlayıcısı): {e}");
                    }
                }
                None => tracing::warn!("ekranlar: ikili yolu güvensiz; bekçi yok (yalnızca shell zamanlayıcısı)"),
            }
        }
        if let Err(e) = self.send(&cmd) {
            if self.restore(&previous, kind) {
                self.files.remove_pending();
            }
            self.set_last(&token, "not_applied", Some(e.clone()));
            return Err(ApiError::new("not_applied", e));
        }
        if let Err(k) = self.verify(&proposed) {
            if self.restore(&previous, kind) {
                self.files.remove_pending();
            }
            self.set_last(&token, "not_applied", Some(k.clone()));
            return Err(ApiError::new("not_applied", format!("{k}: Hyprland istenen durumu uygulamadı; geri dönüldü")));
        }
        self.pending = Some(Pending {
            token: token.clone(),
            previous,
            proposed,
            set_key,
            kind,
            deadline: now.mono + Duration::from_millis(confirm_ms),
        });
        self.r#gen += 1;
        Ok(Applied { token, confirm: true, deadline_ms: confirm_ms })
    }

    /// Önceki yerleşimi zamanlayıcısız uygular (Lua: kurallardan SONRA bekleyen bekçi iptal
    /// edilir). Dönüş: Hyprland kabul etti mi. Başarısızsa çağıran bekleyen kaydı silmez: Lua'da
    /// zamanlayıcı, hyprlang'da bekçi (`reload`) ve açılış kurtarması geri dönüşü üstlenir.
    fn restore(&mut self, previous: &DisplayLayout, kind: ConfigKind) -> bool {
        let Some(hw) = self.hw.clone() else { return false };
        let ok = match runtime_command(previous, &hw, kind, None) {
            Some(cmd) => match self.send(&cmd) {
                Ok(()) => true,
                Err(e) => {
                    tracing::warn!("ekranlar: geri dönüş uygulanamadı: {e}");
                    false
                }
            },
            None => {
                tracing::warn!("ekranlar: geri dönüş kuralları üretilemedi");
                false
            }
        };
        let _ = self.refresh();
        ok
    }

    /// Kalıcı yazım: küme kaydı + üretilen Hyprland dosyası. Hata loglanır.
    fn persist(&mut self, set_key: &[String], layout: &DisplayLayout, kind: ConfigKind) {
        self.store.put(set_key.to_vec(), layout.clone());
        if let Err(e) = self.files.write_store(&self.store.to_toml()) {
            tracing::warn!("ekranlar: displays.toml yazılamadı: {e}");
        }
        if let Some(hw) = &self.hw
            && let Err(e) = self.files.write_generated(kind, &file_text(layout, hw, kind), true)
        {
            tracing::warn!("ekranlar: üretilen Hyprland dosyası yazılamadı: {e}");
        }
    }

    fn take_pending(&mut self, token: &str) -> Result<Pending, ApiError> {
        if !valid_token(token) {
            return Err(ApiError::new("bad_json", "geçersiz jeton"));
        }
        match self.pending.take() {
            Some(p) if p.token == token => Ok(p),
            Some(p) => {
                self.pending = Some(p);
                Err(ApiError::new("stale", "jeton bekleyen değişikliğin değil"))
            }
            None => Err(ApiError::new("stale", "onay bekleyen değişiklik yok")),
        }
    }

    pub fn confirm(&mut self, token: &str) -> Result<(), ApiError> {
        let p = self.take_pending(token)?;
        if p.kind == ConfigKind::Lua
            && let Err(e) = self.send(&format!("eval {}", lua_disarm()))
        {
            tracing::warn!("ekranlar: Hyprland zamanlayıcısı iptal edilemedi: {e}");
        }
        // Önce kalıcı yazım, sonra kayıt silinir (hyprlang bekçisi kaydı bulamayınca çıkar).
        self.persist(&p.set_key, &p.proposed, p.kind);
        self.files.remove_pending();
        self.r#gen += 1;
        self.set_last(&p.token, "confirmed", None);
        Ok(())
    }

    pub fn revert(&mut self, token: &str) -> Result<(), ApiError> {
        let p = self.take_pending(token)?;
        self.revert_pending(p, "reverted");
        Ok(())
    }

    fn revert_pending(&mut self, p: Pending, result: &'static str) {
        let restored = self.restore(&p.previous, p.kind);
        if restored {
            self.files.remove_pending();
        }
        self.r#gen += 1;
        let detail = (!restored).then(|| "geri dönüş Hyprland'e gönderilemedi; Hyprland tarafındaki bekçi geri dönecek".to_owned()); // i18n: tanı msg last ayrıntısı (msg JSON)
        self.set_last(&p.token, result, detail);
    }

    /// Zamanlayıcı: süre dolduysa geri döner. Dönüş: geri dönüldü mü.
    pub fn tick(&mut self, now: Instant) -> bool {
        match self.pending.take() {
            Some(p) if now >= p.deadline => {
                tracing::info!("ekranlar: onay süresi doldu; geri dönülüyor");
                self.revert_pending(p, "timeout");
                true
            }
            other => {
                self.pending = other;
                false
            }
        }
    }

    /// Hyprland monitör durumu değişmiş olabilir (socket2). Bağlı küme değiştiyse: bekleyen
    /// değişiklikten geri dönülür; kayıtlı küme onaysız uygulanır; üretilen dosya bu küme için
    /// yeniden yazılır. Dönüş: küme değişti mi.
    pub fn on_monitors_changed(&mut self) -> bool {
        if let Err(e) = self.refresh() {
            tracing::debug!("ekranlar: monitörler okunamadı: {e}");
            return false;
        }
        let current = self.hw.as_ref().map(Hardware::set_key);
        if current == self.handled_set {
            return false;
        }
        self.handled_set = current;
        if let Some(p) = self.pending.take() {
            tracing::info!("ekranlar: onay beklerken monitör takıldı/çıkarıldı; geri dönülüyor");
            self.revert_pending(p, "hotplug");
        }
        self.apply_saved_set();
        true
    }

    /// Şimdiki küme kayıtlıysa (daha önce onaylanmış) onaysız uygular ve dosyayı yeniler;
    /// değilse üretilen dosya (varsa) boşaltılır — bilinmeyen kümede Hyprland'in/kullanıcının
    /// kendi kuralları geçerlidir (ADR-0005 §8).
    fn apply_saved_set(&mut self) {
        let (Some(hw), Some(kind)) = (self.hw.clone(), self.provider) else { return };
        let key = hw.set_key();
        let record = self.store.get(&key).map(|r| r.layout.clone());
        let layout = match record.map(|l| validate(&l, &hw)) {
            Some(Ok(l)) => l,
            Some(Err(issues)) => {
                tracing::warn!("ekranlar: kayıtlı yerleşim artık geçersiz ({}); uygulanmadı", issue_error(&issues).line());
                let _ = self.files.write_generated(kind, &file_text(&DisplayLayout::default(), &hw, kind), false);
                self.ensure_active_display(&hw, kind);
                return;
            }
            None => {
                let _ = self.files.write_generated(kind, &file_text(&DisplayLayout::default(), &hw, kind), false);
                self.ensure_active_display(&hw, kind);
                return;
            }
        };
        if self.writer.is_some() && mismatch(&layout, &hw).is_some() {
            match runtime_command(&layout, &hw, kind, None) {
                Some(cmd) => match self.send(&cmd) {
                    Ok(()) => tracing::info!("ekranlar: kayıtlı küme yerleşimi uygulandı"),
                    Err(e) => tracing::warn!("ekranlar: kayıtlı yerleşim uygulanamadı: {e}"),
                },
                None => tracing::warn!("ekranlar: kayıtlı yerleşimin kuralları üretilemedi"),
            }
            let _ = self.refresh();
        }
        self.store.touch(&key);
        if let Err(e) = self.files.write_store(&self.store.to_toml()) {
            tracing::warn!("ekranlar: displays.toml yazılamadı: {e}");
        }
        if let Err(e) = self.files.write_generated(kind, &file_text(&layout, &hw, kind), true) {
            tracing::warn!("ekranlar: üretilen Hyprland dosyası yazılamadı: {e}");
        }
    }

    /// Bilinmeyen/geçersiz kümede Nuxut'un önceki çalışma anı kuralları (ör. "dizüstü ekranı
    /// kapalı") kalabilir; dosyayı boşaltmak bunları kaldırmaz. Hiç etkin, yansıtmayan monitör
    /// yoksa kapalı/yansıtan monitörler Hyprland varsayılanıyla açılır (siyah ekran olmaz).
    fn ensure_active_display(&mut self, hw: &Hardware, kind: ConfigKind) {
        if self.writer.is_none() || hw.monitors.iter().any(|m| m.enabled && m.mirror_of.is_none()) {
            return;
        }
        let Some(cmd) = fallback_command(hw, kind) else { return };
        match self.send(&cmd) {
            Ok(()) => tracing::warn!("ekranlar: etkin ekran kalmamıştı; monitörler varsayılanla açıldı"),
            Err(e) => tracing::warn!("ekranlar: etkin ekran yok ve açılamadı: {e}"),
        }
        let _ = self.refresh();
    }

    /// Açılış: yarım kalan (onaylanmamış) değişiklik varsa geri dönülür; sonra şimdiki küme
    /// kayıtlıysa uygulanır.
    pub fn startup(&mut self) {
        if let Err(e) = self.refresh() {
            tracing::info!("ekranlar: Hyprland okunamadı: {e}");
            return;
        }
        if let Some(rec) = self.files.read_pending() {
            let kind = match rec.provider.as_str() {
                "lua" => Some(ConfigKind::Lua),
                "hyprlang" => Some(ConfigKind::Hyprlang),
                _ => None,
            };
            match (kind, parse_apply(&rec.previous)) {
                (Some(kind), Ok(req)) if self.writer.is_some() => {
                    tracing::info!("ekranlar: onaylanmamış değişiklik bulundu; geri dönülüyor");
                    if self.restore(&req.layout, kind) {
                        self.files.remove_pending();
                        self.set_last(&rec.token, "reverted", Some("shell yeniden başladı".into()));
                    }
                }
                (Some(_), Ok(_)) => {}
                _ => {
                    tracing::warn!("ekranlar: bekleyen değişiklik kaydı okunamadı; yok sayıldı");
                    self.files.remove_pending();
                }
            }
        }
        self.apply_saved_set();
        self.handled_set = self.hw.as_ref().map(Hardware::set_key);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use super::super::model::fixtures::DUAL;
    use super::*;

    /// Sahte Hyprland: `j/monitors all` için şimdiki durum; her yazma komutu (eval/keyword/
    /// dispatch) kaydedilir ve sıradaki durum (varsa) şimdiki olur.
    #[derive(Default)]
    pub(crate) struct FakeHypr {
        pub monitors: RefCell<String>,
        pub after_write: RefCell<VecDeque<String>>,
        pub sent: RefCell<Vec<String>>,
        pub provider: &'static str,
        pub reject: RefCell<bool>,
    }

    impl HyprControl for Rc<FakeHypr> {
        fn request(&self, command: &str) -> Result<String, String> {
            match command {
                "j/monitors all" => Ok(self.monitors.borrow().clone()),
                "j/status" => Ok(format!("{{\"configProvider\":\"{}\"}}", self.provider)),
                c => {
                    self.sent.borrow_mut().push(c.to_owned());
                    if *self.reject.borrow() {
                        return Ok("error: nope".into());
                    }
                    if !c.starts_with("dispatch exec")
                        && let Some(next) = self.after_write.borrow_mut().pop_front()
                    {
                        *self.monitors.borrow_mut() = next;
                    }
                    Ok(if c.starts_with("[[BATCH]]") { "ok\n\nok".into() } else { "ok".into() })
                }
            }
        }
    }

    #[derive(Default)]
    pub(crate) struct MemFiles {
        pub store: RefCell<Option<String>>,
        pub generated: RefCell<Option<(ConfigKind, String)>>,
        pub pending: RefCell<Option<PendingRecord>>,
    }

    impl DisplayFiles for Rc<MemFiles> {
        fn read_store(&self) -> Option<String> {
            self.store.borrow().clone()
        }
        fn write_store(&self, text: &str) -> Result<(), String> {
            *self.store.borrow_mut() = Some(text.into());
            Ok(())
        }
        fn write_generated(&self, kind: ConfigKind, text: &str, create: bool) -> Result<(), String> {
            if create || self.generated.borrow().is_some() {
                *self.generated.borrow_mut() = Some((kind, text.into()));
            }
            Ok(())
        }
        fn read_pending(&self) -> Option<PendingRecord> {
            self.pending.borrow().clone()
        }
        fn write_pending(&self, r: &PendingRecord) -> Result<(), String> {
            *self.pending.borrow_mut() = Some(r.clone());
            Ok(())
        }
        fn remove_pending(&self) {
            *self.pending.borrow_mut() = None;
        }
    }

    const DELL: &str = "Dell Inc.|DELL U2720Q|ABC123";

    pub(crate) fn setup(provider: &'static str, control: bool) -> (DisplayManager, Rc<FakeHypr>, Rc<MemFiles>) {
        let fake = Rc::new(FakeHypr { provider, ..Default::default() });
        *fake.monitors.borrow_mut() = DUAL.into();
        let files = Rc::new(MemFiles::default());
        let opts = Options { exe: "/usr/bin/nuxut-shell".into(), verify_tries: 2, verify_interval: Duration::ZERO, ..Options::default() };
        let writer: Option<Box<dyn HyprControl>> = control.then(|| Box::new(Rc::clone(&fake)) as Box<dyn HyprControl>);
        let m = DisplayManager::new(opts, Box::new(Rc::clone(&fake)), writer, Box::new(Rc::clone(&files)));
        (m, fake, files)
    }

    fn now() -> Now {
        Now { mono: Instant::now(), unix_ms: 1_000_000 }
    }

    fn list(m: &mut DisplayManager) -> serde_json::Value {
        serde_json::from_str(&m.list(Instant::now()).unwrap()).unwrap()
    }

    /// Dell'in ölçeğini değiştiren istek + Hyprland'in uyguladıktan sonraki durumu.
    fn scale_change(m: &mut DisplayManager) -> (String, String) {
        let r#gen = list(m)["gen"].as_u64().unwrap();
        let hw = Hardware::parse(DUAL).unwrap();
        let mut l = hw.current_layout();
        let d = l.displays.iter_mut().find(|d| d.key == DELL).unwrap();
        d.scale_120 = 150; // 2560x1440 / 1.25 = 2048x1152
        (apply_json(r#gen, &l), DUAL.replace("\"scale\":1,", "\"scale\":1.25,"))
    }

    #[test]
    fn list_and_status_shape() {
        let (mut m, _, _) = setup("lua", true);
        let v = list(&mut m);
        assert_eq!(v["v"], 1);
        assert_eq!(v["provider"], "lua");
        assert_eq!(v["available"], true);
        assert_eq!(v["displays"].as_array().unwrap().len(), 2);
        let s: serde_json::Value = serde_json::from_str(&m.status(Instant::now())).unwrap();
        assert_eq!(s["gen"], v["gen"]);
        assert!(s["pending"].is_null());
        // Aynı durum yeniden okununca gen değişmez.
        assert_eq!(list(&mut m)["gen"], v["gen"]);
    }

    #[test]
    fn confirm_flow_lua() {
        let (mut m, fake, files) = setup("lua", true);
        let (json, after) = scale_change(&mut m);
        fake.after_write.borrow_mut().push_back(after);
        let t = now();
        let a = m.apply(&json, t).unwrap();
        assert!(a.confirm);
        assert_eq!(a.deadline_ms, 15_000);
        let sent = fake.sent.borrow().clone();
        assert_eq!(sent.len(), 1, "tek eval: {sent:?}");
        assert!(sent[0].starts_with("eval ") && sent[0].contains("NUXUT_DISPLAY_GUARD = hl.timer(") && sent[0].contains("timeout = 20000"));
        assert_eq!(files.pending.borrow().as_ref().unwrap().token, a.token);
        assert!(files.store.borrow().is_none(), "onaydan önce kalıcı yazım yok");
        // İkinci apply reddedilir.
        assert_eq!(m.apply(&json, t).unwrap_err().code, "pending_exists");
        let s: serde_json::Value = serde_json::from_str(&m.status(t.mono)).unwrap();
        assert_eq!(s["pending"]["token"], a.token.as_str());
        // Yanlış jeton.
        assert_eq!(m.confirm("abcdef").unwrap_err().code, "stale");
        m.confirm(&a.token).unwrap();
        assert!(fake.sent.borrow().last().unwrap().contains("set_enabled(false)"), "zamanlayıcı iptal");
        assert!(files.pending.borrow().is_none());
        let store = files.store.borrow().clone().unwrap();
        assert!(store.contains("scale = 1.25"), "{store}");
        let (kind, text) = files.generated.borrow().clone().unwrap();
        assert_eq!(kind, ConfigKind::Lua);
        assert!(text.contains("scale = 1.25"));
        assert!(!m.is_pending());
        let s: serde_json::Value = serde_json::from_str(&m.status(t.mono)).unwrap();
        assert_eq!(s["last"]["result"], "confirmed");
    }

    #[test]
    fn timeout_reverts_to_previous() {
        let (mut m, fake, files) = setup("lua", true);
        let (json, after) = scale_change(&mut m);
        fake.after_write.borrow_mut().extend([after, DUAL.to_owned()]);
        let t = now();
        m.apply(&json, t).unwrap();
        assert!(!m.tick(t.mono + Duration::from_secs(14)));
        assert!(m.tick(t.mono + Duration::from_secs(15)));
        let last = fake.sent.borrow().last().unwrap().clone();
        assert!(last.starts_with("eval hl.monitor(") && last.ends_with("NUXUT_DISPLAY_GUARD = nil end"), "geri dönüş: önce kurallar, sonra bekçi iptali: {last}");
        assert!(last.contains("scale = 1,") && !last.contains("hl.timer("), "{last}");
        assert!(files.pending.borrow().is_none());
        assert!(files.store.borrow().is_none());
        let s: serde_json::Value = serde_json::from_str(&m.status(t.mono)).unwrap();
        assert_eq!(s["last"]["result"], "timeout");
    }

    #[test]
    fn revert_and_hotplug() {
        let (mut m, fake, _) = setup("lua", true);
        let (json, after) = scale_change(&mut m);
        fake.after_write.borrow_mut().push_back(after.clone());
        let a = m.apply(&json, now()).unwrap();
        m.revert(&a.token).unwrap();
        assert_eq!(m.revert(&a.token).unwrap_err().code, "stale");
        // Yeniden uygula; beklerken Dell çıkarılır → geri dönüş.
        *fake.monitors.borrow_mut() = DUAL.into();
        let (json, after) = scale_change(&mut m);
        fake.after_write.borrow_mut().push_back(after);
        m.apply(&json, now()).unwrap();
        let only_edp: serde_json::Value = serde_json::from_str(DUAL).unwrap();
        *fake.monitors.borrow_mut() = format!("[{}]", only_edp[0]);
        assert!(m.on_monitors_changed());
        assert!(!m.is_pending());
        let s: serde_json::Value = serde_json::from_str(&m.status(Instant::now())).unwrap();
        assert_eq!(s["last"]["result"], "hotplug");
    }

    #[test]
    fn position_only_is_applied_without_confirm() {
        let (mut m, fake, files) = setup("hyprlang", true);
        let r#gen = list(&mut m)["gen"].as_u64().unwrap();
        let hw = Hardware::parse(DUAL).unwrap();
        let mut l = hw.current_layout();
        l.displays.iter_mut().find(|d| d.key == DELL).unwrap().pos = (1920, 600);
        fake.after_write.borrow_mut().push_back(DUAL.replace("\"x\":1920,\"y\":0", "\"x\":1920,\"y\":600"));
        let a = m.apply(&apply_json(r#gen, &l), now()).unwrap();
        assert!(!a.confirm);
        assert!(fake.sent.borrow()[0].starts_with("[[BATCH]]keyword monitor "));
        assert!(files.pending.borrow().is_none());
        assert!(files.store.borrow().as_ref().unwrap().contains("position = [1920, 600]"));
        assert_eq!(files.generated.borrow().as_ref().unwrap().0, ConfigKind::Hyprlang);
    }

    #[test]
    fn hyprlang_starts_external_guard() {
        let (mut m, fake, files) = setup("hyprlang", true);
        let (json, after) = scale_change(&mut m);
        fake.after_write.borrow_mut().push_back(after);
        let a = m.apply(&json, now()).unwrap();
        let sent = fake.sent.borrow().clone();
        let deadline = files.pending.borrow().as_ref().unwrap().deadline_unix_ms;
        assert_eq!(deadline, 1_000_000 + 20_000);
        assert_eq!(sent[0], format!("dispatch exec /usr/bin/nuxut-shell display-guard {} {deadline} nuxut-shell", a.token));
        assert!(sent[1].starts_with("[[BATCH]]keyword monitor "));
        m.confirm(&a.token).unwrap();
        assert_eq!(fake.sent.borrow().len(), 2, "hyprlang'da onay Hyprland'e komut göndermez (kayıt silinir)");
        assert!(files.pending.borrow().is_none());
    }

    #[test]
    fn not_applied_rolls_back() {
        let (mut m, fake, files) = setup("lua", true);
        let (json, _) = scale_change(&mut m);
        // Hyprland ölçeği uygulamadı (durum aynı kaldı).
        let e = m.apply(&json, now()).unwrap_err();
        assert_eq!(e.code, "not_applied");
        assert!(e.message.contains(DELL));
        assert_eq!(fake.sent.borrow().len(), 2, "uygula + geri dön");
        assert!(files.pending.borrow().is_none() && !m.is_pending());
        // Hyprland reddederse.
        *fake.reject.borrow_mut() = true;
        let (json, _) = scale_change(&mut m);
        assert_eq!(m.apply(&json, now()).unwrap_err().code, "not_applied");
    }

    #[test]
    fn rejects_without_control_and_on_bad_input() {
        let (mut m, fake, _) = setup("lua", false);
        let (json, _) = scale_change(&mut m);
        assert_eq!(m.apply(&json, now()).unwrap_err().code, "unavailable");
        assert!(fake.sent.borrow().is_empty());
        let (mut m, fake, _) = setup("lua", true);
        let (json, _) = scale_change(&mut m);
        assert_eq!(m.apply(&json.replace("\"base_gen\":1", "\"base_gen\":99"), now()).unwrap_err().code, "stale");
        assert_eq!(m.apply(&json.replace("\"scale_120\":150", "\"scale_120\":210"), now()).unwrap_err().code, "invalid_scale");
        let off = json.replace("\"enabled\":true", "\"enabled\":false");
        assert_eq!(m.apply(&off, now()).unwrap_err().code, "no_active_display");
        assert!(fake.sent.borrow().is_empty(), "geçersiz istek Hyprland'e gitmez");
    }

    #[test]
    fn hotplug_applies_saved_set_and_startup_recovers() {
        let (mut m, fake, files) = setup("lua", true);
        let (json, after) = scale_change(&mut m);
        fake.after_write.borrow_mut().push_back(after);
        let a = m.apply(&json, now()).unwrap();
        m.confirm(&a.token).unwrap();
        let store = files.store.borrow().clone();
        // Yeni yönetici aynı dosyalarla: Dell çıkar, sonra Hyprland'in varsayılanıyla (ölçek 1) geri gelir.
        let only_edp: serde_json::Value = serde_json::from_str(DUAL).unwrap();
        *fake.monitors.borrow_mut() = format!("[{}]", only_edp[0]);
        fake.sent.borrow_mut().clear();
        let mem = Rc::new(MemFiles { store: RefCell::new(store), ..Default::default() });
        let mut m2 = DisplayManager::new(
            Options { verify_tries: 1, verify_interval: Duration::ZERO, ..Options::default() },
            Box::new(Rc::clone(&fake)),
            Some(Box::new(Rc::clone(&fake))),
            Box::new(Rc::clone(&mem)),
        );
        m2.startup();
        assert!(fake.sent.borrow().is_empty(), "bilinmeyen küme: dokunulmaz");
        assert!(mem.generated.borrow().is_none(), "dosya yoksa oluşturulmaz");
        *fake.monitors.borrow_mut() = DUAL.into();
        assert!(m2.on_monitors_changed());
        let sent = fake.sent.borrow().clone();
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert!(sent[0].contains("scale = 1.25") && !sent[0].contains("hl.timer("));
        assert!(mem.generated.borrow().as_ref().unwrap().1.contains("scale = 1.25"));

        // Açılışta yarım kalan değişiklik: önceki yerleşim uygulanır, kayıt silinir.
        let (mut m3, fake3, files3) = setup("lua", true);
        let prev = Hardware::parse(DUAL).unwrap().current_layout();
        *files3.pending.borrow_mut() =
            Some(PendingRecord { v: 1, token: "ab".into(), deadline_unix_ms: 0, provider: "lua".into(), previous: apply_json(0, &prev) });
        m3.startup();
        assert!(files3.pending.borrow().is_none());
        assert_eq!(fake3.sent.borrow().len(), 1);
        assert!(fake3.sent.borrow()[0].ends_with("NUXUT_DISPLAY_GUARD = nil end"));
    }

    /// eDP'nin kapalı olduğu onaylanmıştı, harici çıkarıldı: bilinmeyen küme ve hiç etkin ekran
    /// yok → monitörler Hyprland varsayılanıyla açılır (dosyayı boşaltmak yetmez).
    #[test]
    fn unknown_set_without_active_display_turns_screens_on() {
        for (provider, expect) in [
            ("lua", "hl.monitor({ output = \"eDP-1\", mode = \"preferred\", position = \"auto\", scale = \"auto\" })"),
            ("hyprlang", "[[BATCH]]keyword monitor eDP-1, preferred, auto, auto"),
        ] {
            let (mut m, fake, files) = setup(provider, true);
            *files.generated.borrow_mut() = Some((ConfigKind::Lua, "eski".into()));
            let only_edp: serde_json::Value = serde_json::from_str(DUAL).unwrap();
            *fake.monitors.borrow_mut() = format!("[{}]", only_edp[0]).replace("\"disabled\":false", "\"disabled\":true");
            assert!(m.on_monitors_changed());
            let sent = fake.sent.borrow().clone();
            assert_eq!(sent.len(), 1, "{sent:?}");
            assert!(sent[0].contains(expect), "{}", sent[0]);
            assert!(!files.generated.borrow().as_ref().unwrap().1.contains("hl.monitor("), "dosya boşaltıldı");
            // Etkin ekran varsa kurtarma komutu gönderilmez.
            let (mut m, fake, _) = setup(provider, true);
            *fake.monitors.borrow_mut() = format!("[{}]", only_edp[0]);
            assert!(m.on_monitors_changed());
            assert!(fake.sent.borrow().is_empty());
        }
    }

    #[test]
    fn failed_restore_keeps_pending_record() {
        let (mut m, fake, files) = setup("hyprlang", true);
        let (json, after) = scale_change(&mut m);
        fake.after_write.borrow_mut().push_back(after);
        let a = m.apply(&json, now()).unwrap();
        *fake.reject.borrow_mut() = true;
        m.revert(&a.token).unwrap();
        assert_eq!(files.pending.borrow().as_ref().map(|r| r.token.clone()), Some(a.token.clone()), "bekçi/açılış geri dönsün");
        let s: serde_json::Value = serde_json::from_str(&m.status(Instant::now())).unwrap();
        assert_eq!(s["last"]["result"], "reverted");
        assert!(s["last"]["detail"].is_string());
        // Açılışta da geri dönüş başarısızsa kayıt kalır; başarılı olunca silinir.
        let mut m2 = DisplayManager::new(
            Options { verify_tries: 1, verify_interval: Duration::ZERO, ..Options::default() },
            Box::new(Rc::clone(&fake)),
            Some(Box::new(Rc::clone(&fake))),
            Box::new(Rc::clone(&files)),
        );
        m2.startup();
        assert!(files.pending.borrow().is_some());
        *fake.reject.borrow_mut() = false;
        m2.startup();
        assert!(files.pending.borrow().is_none());
    }

    #[test]
    fn set_change_is_not_consumed_by_other_refreshes() {
        let (mut m, fake, _) = setup("lua", true);
        m.startup();
        let only_edp: serde_json::Value = serde_json::from_str(DUAL).unwrap();
        *fake.monitors.borrow_mut() = format!("[{}]", only_edp[0]);
        // Ayarlar `list` yoklaması hotplug olayından önce gelir.
        list(&mut m);
        assert!(m.on_monitors_changed(), "küme değişimi list tarafından yutulmamalı");
        assert!(!m.on_monitors_changed());
    }

    #[test]
    fn confirm_classification() {
        let hw = Hardware::parse(DUAL).unwrap();
        let a = hw.current_layout();
        let mut b = a.clone();
        b.displays[1].pos = (1920, 100);
        b.primary = Some(DELL.into());
        assert!(!needs_confirm(&a, &b));
        b.displays[1].vrr = super::super::model::Vrr::Fullscreen;
        assert!(needs_confirm(&a, &b));
        let mut c = a.clone();
        c.displays[0].enabled = false;
        assert!(needs_confirm(&a, &c));
        assert!(reply_ok("ok") && reply_ok("ok\n\nok") && !reply_ok("") && !reply_ok("error: x"));
    }
}
