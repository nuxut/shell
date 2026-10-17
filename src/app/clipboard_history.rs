//! Pano geçmişi bağlantısı (t060; `services::clipboard`): servis, gizlilik bağlamı, framework
//! data-control izlemesi, başlatıcının pano kipi ve `msg clipboard …`.
//!
//! İzleme framework'ün data-control yeteneğidir (`cx.data_control_start`). Compositor sunmuyorsa
//! özellik tek bilgi satırıyla kapalı kalır (`available: false`, panik yok). İzleme durursa
//! (`Stopped`) geçmiş korunur, `available: false` yayınlanır ve geri çekilmeli yeniden denenir
//! ([`Restart`]). `clipboard.enabled = false` iken geçmiş için izleme başlatılmaz; VM pano köprüsü
//! devredeyse yalnız pano izlenir ve geçmiş filtresi `Disabled` ile hiçbir teklifi okumaz.
//! Birincil seçim yalnızca `clipboard.primary` (ve geçmiş açık) ile izlenir.

use std::sync::Arc;
use std::time::{Instant, SystemTime};

use nuxut::clipboard::Selection;
use nuxut::{Context, DataControlError, DataControlEvent, SurfaceId, TimerId};

use super::{Shell, ShellMessage};
use crate::launcher_view::THUMB_SIDE;
use crate::services::clipboard::source::{DataControlSource, Restart, Translated};
use crate::services::clipboard::takeover::{self, TakeoverResult};
use crate::services::clipboard::x11::{BridgeEvent, BridgeMode, X11Bridge};
use crate::services::hypr_config::sync::SyncEnv;
use crate::services::clipboard::{
    ClipboardCommand, ClipboardEvent, ClipboardService, ClipboardSnapshot, ClipboardSource, Gate, Inbox, ServiceOptions, SourceEvent, SourceSink,
    channel, store,
};
use crate::widgets::{Action, PanelId};

/// Servis, paylaşılan gizlilik bağlamı ve framework izlemesi.
#[derive(Debug, Default)]
pub(super) struct ClipboardRuntime {
    service: Option<ClipboardService>,
    gate: Arc<Gate>,
    /// Framework adaptörü (yalnızca compositor data-control sunuyorsa).
    source: Option<Arc<DataControlSource>>,
    /// Servisin olay ucu (adaptör olayları ve `available`).
    sink: Option<SourceSink>,
    restart: Restart,
    retry_timer: Option<TimerId>,
    /// VM pano köprüsü (`[clipboard] x11_bridge`; yalnızca data-control varken) ve kurulduğu kip.
    bridge: Option<(BridgeMode, X11Bridge)>,
    /// Köprü devrede (`auto`'da konuk ajanı bulundu): geçmiş kapalı olsa da pano izlenir.
    bridge_active: bool,
    /// Kullanıcı betiğinin devralması bu süreçte denendi.
    takeover_tried: bool,
    /// Köprü Xwayland'e bağlandı (devralmanın ikinci koşulu data-control izlemesi).
    bridge_connected: bool,
}

/// Kaynak: canlı görsel test derlemesinde demo (`--features clipboard-demo`, üründe yok),
/// yoksa framework data-control (compositor sunuyorsa).
enum Connected {
    #[cfg_attr(not(feature = "clipboard-demo"), allow(dead_code))]
    Demo(Arc<dyn ClipboardSource>, Inbox),
    DataControl(Arc<DataControlSource>, SourceSink, Inbox),
}

fn connect_source(cx: &Context<Shell>) -> Option<Connected> {
    #[cfg(feature = "clipboard-demo")]
    if let Some((source, inbox)) = crate::services::clipboard::demo::connect() {
        return Some(Connected::Demo(source, inbox));
    }
    let protocol = cx.data_control_protocol()?;
    tracing::info!(?protocol, "pano geçmişi: data-control");
    let (sink, inbox) = channel();
    Some(Connected::DataControl(Arc::new(DataControlSource::default()), sink, inbox))
}

/// Küçük resim çözme ayarları: güvenilmeyen bayt (sınırlar `Limits::ICON` tabanlı, kodlu
/// boyut görsel öğesi sınırıyla), en uzun kenar [`THUMB_SIDE`], yalnızca ilk kare.
pub(super) fn thumb_options() -> nuxut_image::DecodeOptions {
    use nuxut_image::{DecodeOptions, Fit, Limits, TargetSize, Trust};
    DecodeOptions {
        limits: Limits {
            max_file_bytes: crate::services::clipboard::history::MAX_IMAGE_BYTES as u64,
            max_width: 8_192,
            max_height: 8_192,
            max_pixels: 40_000_000,
            // Açıkça: pano baytları başka uygulamadan gelir, yalnızca yalıtılmış işçide çözülür.
            trust: Trust::Untrusted,
            ..Limits::ICON
        },
        target: Some(TargetSize::new(THUMB_SIDE, THUMB_SIDE, Fit::Contain)),
        animate: false,
    }
}

impl Shell {
    pub(super) fn start_clipboard(&mut self, cx: &mut Context<Self>) {
        self.set_clipboard_unavailable();
        let Some(connected) = connect_source(cx) else {
            tracing::info!("pano geçmişi: compositor data-control sunmuyor; kapalı");
            return;
        };
        let (source, inbox): (Arc<dyn ClipboardSource>, Inbox) = match connected {
            Connected::Demo(source, inbox) => (source, inbox),
            Connected::DataControl(adapter, sink, inbox) => {
                self.clipboard.source = Some(adapter.clone());
                self.clipboard.sink = Some(sink);
                (adapter, inbox)
            }
        };
        self.clipboard.gate.set_exclude_apps(&self.model.config.clipboard.exclude_apps);
        self.sync_clipboard_gate();
        let sender = cx.sender();
        // Disk yazımı yalnızca yazım ortamı açıkken (test örneği kendi geçici XDG_STATE_HOME'una).
        let persist_path = (!self.apply_env.disabled).then(|| self.apply_env.state_home.as_deref().map(store::path_in)).flatten();
        let options = ServiceOptions { config: self.model.config.clipboard.clone(), persist_path };
        match ClipboardService::spawn(inbox, source, self.clipboard.gate.clone(), options, move |e| {
            let _ = sender.send(ShellMessage::Clipboard(e));
        }) {
            Ok(s) => self.clipboard.service = Some(s),
            Err(e) => {
                tracing::warn!("pano servisi başlatılamadı: {e}");
                self.clipboard.source = None;
                self.clipboard.sink = None;
                return;
            }
        }
        self.sync_clipboard_bridge(cx);
        self.clipboard_watch(cx);
    }

    /// VM pano köprüsünü config'e uydurur: kip değiştiyse eskisi durur (X11 seçimi bırakılır),
    /// yenisi kurulur. Köprü yalnızca framework data-control kaynağıyla çalışır. `auto`'da
    /// köprü konuk ajanını bulunca [`BridgeEvent::Active`] gelir ve izleme başlar.
    fn sync_clipboard_bridge(&mut self, cx: &mut Context<Self>) {
        let mode = self.model.config.clipboard.x11_bridge;
        let Some(source) = self.clipboard.source.clone().filter(|_| mode != BridgeMode::Off) else {
            self.clipboard.bridge = None;
            self.clipboard.bridge_connected = false;
            self.clipboard.bridge_active = false;
            return;
        };
        if self.clipboard.bridge.as_ref().is_some_and(|(m, _)| *m == mode) {
            return;
        }
        // Önce eskisi: aynı anda iki X11 sahibi olmasın.
        self.clipboard.bridge = None;
        self.clipboard.bridge_connected = false;
        self.clipboard.bridge_active = false;
        let sender = cx.sender();
        match X11Bridge::spawn(mode, std::env::var_os("DISPLAY"), source, self.clipboard.gate.clone(), move |e| {
            let _ = sender.send(ShellMessage::ClipboardBridge(e));
        }) {
            Ok(b) => self.clipboard.bridge = Some((mode, b)),
            Err(e) => tracing::warn!("pano köprüsü başlatılamadı: {e}"),
        }
    }

    pub(super) fn on_clipboard_bridge(&mut self, cx: &mut Context<Self>, e: BridgeEvent) {
        match e {
            BridgeEvent::Active => {
                if self.clipboard.bridge.is_none() {
                    return;
                }
                self.clipboard.bridge_active = true;
            }
            BridgeEvent::Connected => {
                self.clipboard.bridge_connected = true;
                self.clipboard_bridge_takeover();
                return;
            }
        }
        self.clipboard_watch(cx);
    }

    /// Kullanıcının kendi pano köprüsü betiğinin çalışma anı devralması
    /// (`services::clipboard::takeover`): yalnızca Nuxut köprüsü Xwayland'e bağlıyken ve
    /// data-control izlemesi çalışırken, süreç başına bir kez, thread'de (dosya IO'su). Kullanıcı
    /// hiçbir girişte panosuz kalmaz: satır, köprü gerçekten çalıştıktan sonra kapatılır;
    /// çalışan betik sürecine dokunulmaz (bir sonraki girişte satır çalışmaz).
    fn clipboard_bridge_takeover(&mut self) {
        let attached = self.clipboard.source.as_ref().is_some_and(|s| s.attached());
        if self.clipboard.bridge.is_none() || !self.clipboard.bridge_connected || !attached || std::mem::replace(&mut self.clipboard.takeover_tried, true) {
            return;
        }
        let env = SyncEnv::from_process(self.apply_env.clone());
        let spawned = std::thread::Builder::new().name("nuxut-clip-takeover".into()).spawn(move || match takeover::take_over(&env) {
            TakeoverResult::Disabled(n) => tracing::info!(lines = n, "pano köprüsü: kullanıcının köprü betiği satırı yedekli kapatıldı (geri al: msg hypr undo)"),
            TakeoverResult::Failed(e) => tracing::warn!("pano köprüsü: betik devralınamadı ({e})"),
            other => tracing::debug!(?other, "pano köprüsü: betik devralma"),
        });
        if let Err(e) = spawned {
            tracing::warn!("pano devralma thread'i açılamadı: {e}");
        }
    }

    /// `msg hypr undo`: köprü betiği devralmasını da geri alır (thread'de).
    pub(super) fn clipboard_bridge_takeover_undo(&self) {
        let env = SyncEnv::from_process(self.apply_env.clone());
        let spawned = std::thread::Builder::new().name("nuxut-clip-takeover".into()).spawn(move || match takeover::undo(&env) {
            Ok(true) => tracing::info!("pano köprüsü: betik satırı geri yüklendi"),
            Ok(false) => {}
            Err(e) => tracing::warn!("pano köprüsü: betik satırı geri yüklenemedi ({e})"),
        });
        if let Err(e) = spawned {
            tracing::warn!("pano devralma thread'i açılamadı: {e}");
        }
    }

    /// Pano izlenmeli mi: geçmiş açık ya da köprü devrede.
    fn clipboard_wanted(&self) -> bool {
        self.model.config.clipboard.enabled || self.clipboard.bridge_active
    }

    /// İzlemeyi config'e uydurur: geçmiş açık ya da köprü devredeyse başlatır (başlamışsa yalnızca birincil
    /// anahtarı), değilse durdurur. Bekleyen yeniden deneme varsa ona bırakır.
    fn clipboard_watch(&mut self, cx: &mut Context<Self>) {
        let Some(source) = self.clipboard.source.clone() else { return };
        let wanted = self.clipboard_wanted();
        let config = &self.model.config.clipboard;
        // Birincil seçim yalnızca geçmiş için (köprü yalnızca panoyu taşır).
        let primary = config.primary && config.enabled;
        if !wanted {
            if let Some(t) = self.clipboard.retry_timer.take() {
                cx.cancel_timer(t);
            }
            self.clipboard.restart.reset();
            if source.attached() {
                cx.data_control_stop();
                source.detach();
            }
            self.clipboard_available(false);
            return;
        }
        if source.attached() {
            cx.data_control_watch_primary(primary);
            return;
        }
        if self.clipboard.retry_timer.is_some() {
            return;
        }
        match cx.data_control_start() {
            Ok(handle) => {
                source.attach(handle);
                self.clipboard.restart.started(Instant::now());
                // `start`'tan hemen sonra açmak güvenli (framework sözleşmesi).
                cx.data_control_watch_primary(primary);
                self.clipboard_available(true);
                self.clipboard_bridge_takeover();
            }
            Err(DataControlError::Unsupported) => {
                // Açılışta bildirilip sonra kaybolan global: yeniden denemenin anlamı yok.
                tracing::info!("pano geçmişi: data-control desteklenmiyor; kapalı");
                self.clipboard_available(false);
            }
            Err(e @ DataControlError::NoSeat) => {
                self.clipboard_available(false);
                self.clipboard_retry(cx, &e);
            }
        }
    }

    fn clipboard_available(&self, on: bool) {
        if let Some(sink) = &self.clipboard.sink {
            sink.set_available(on);
        }
    }

    fn clipboard_retry(&mut self, cx: &mut Context<Self>, why: &dyn std::fmt::Display) {
        let delay = self.clipboard.restart.failed(Instant::now());
        tracing::info!(delay_s = delay.as_secs(), "pano izleme durdu ({why}); yeniden denenecek");
        self.clipboard.retry_timer = Some(cx.set_timer(delay));
    }

    /// Framework data-control olayı (`App::data_control`).
    pub(super) fn on_data_control(&mut self, cx: &mut Context<Self>, event: DataControlEvent) {
        let (Some(source), Some(sink)) = (self.clipboard.source.clone(), self.clipboard.sink.clone()) else { return };
        // Kuyruktaki eski izlemenin olayı (kapatıldıktan sonra): yok sayılır.
        if !source.attached() {
            return;
        }
        // Teklif ana döngüde değil, servis thread'inde okunur; kilit bağlamı karar anında güncel.
        self.sync_clipboard_gate();
        let clipboard = match &event {
            DataControlEvent::SelectionOffered { selection, .. } | DataControlEvent::SelectionCleared { selection } => {
                *selection == Selection::Clipboard
            }
            _ => false,
        };
        match source.translate(event) {
            Translated::Event(e) => {
                // Köprü yalnızca panoyu taşır (birincil seçim değil); kimlik eşlemesi ortak.
                if clipboard && let Some((_, bridge)) = &self.clipboard.bridge {
                    match &e {
                        SourceEvent::Offered { id, mimes, flags } => bridge.offered(*id, mimes.clone(), *flags),
                        SourceEvent::Cleared => bridge.cleared(),
                    }
                }
                sink.send(e);
            }
            Translated::Ignored => {}
            Translated::Stopped => {
                source.detach();
                self.clipboard_available(false);
                self.clipboard_retry(cx, &"Stopped");
            }
        }
    }

    pub(super) fn clipboard_timer_fired(&mut self, cx: &mut Context<Self>, id: TimerId) -> bool {
        if self.clipboard.retry_timer != Some(id) {
            return false;
        }
        self.clipboard.retry_timer = None;
        self.clipboard_watch(cx);
        true
    }

    /// İzleyici yokken modelin anlık görüntüsü (config'in `enabled`'ı yansır).
    fn set_clipboard_unavailable(&mut self) {
        self.model.clipboard =
            Arc::new(ClipboardSnapshot { enabled: self.model.config.clipboard.enabled, available: false, items: Vec::new() });
    }

    pub(super) fn on_clipboard(&mut self, cx: &mut Context<Self>, e: ClipboardEvent) {
        match e {
            ClipboardEvent::Snapshot(s) => self.model.clipboard = s,
        }
        self.model_changed(cx);
        self.pump_launcher_icons(cx);
    }

    /// Gizlilik bağlamını (oturum kilidi, etkin pencere sınıfı) servis ve köprüyle paylaşır. Her mesajdan
    /// sonra çağrılır; değişmediyse iş yapmaz.
    pub(super) fn sync_clipboard_gate(&self) {
        let class = self.model.hypr.active_window.as_ref().map(|w| w.class.as_str());
        self.clipboard.gate.set(self.idle.session_locked(), class);
    }

    pub(super) fn clipboard_config_changed(&mut self, cx: &mut Context<Self>) {
        self.clipboard.gate.set_exclude_apps(&self.model.config.clipboard.exclude_apps);
        match &self.clipboard.service {
            Some(s) => s.set_config(self.model.config.clipboard.clone()),
            None => {
                self.set_clipboard_unavailable();
                self.model_changed(cx);
            }
        }
        self.sync_clipboard_bridge(cx);
        // Açıldıysa hemen dene (geri çekilme sıfırdan).
        if self.clipboard_wanted() && self.clipboard.source.as_ref().is_some_and(|s| !s.attached()) {
            if let Some(t) = self.clipboard.retry_timer.take() {
                cx.cancel_timer(t);
            }
            self.clipboard.restart.reset();
        }
        self.clipboard_watch(cx);
    }

    /// `msg clipboard …`.
    pub(super) fn clipboard_command(&mut self, c: ClipboardCommand) -> Result<String, String> {
        if let ClipboardCommand::List = c {
            return Ok(self.model.clipboard.list_json(SystemTime::now()));
        }
        let service = self.clipboard.service.as_ref().ok_or("pano geçmişi kullanılamıyor (data-control yok)")?; // i18n: tanı msg yanıtı/bildirim ayrıntısı
        match c {
            ClipboardCommand::List => {}
            ClipboardCommand::Pick(id) | ClipboardCommand::Delete(id) if self.model.clipboard.get(id).is_none() => {
                return Err(format!("not_found: öğe {id} yok")); // i18n: tanı msg yanıtı/bildirim ayrıntısı
            }
            ClipboardCommand::Pick(id) => service.pick(id),
            ClipboardCommand::Delete(id) => service.delete(id),
            ClipboardCommand::Clear => service.clear(),
        }
        Ok("ok".into())
    }

    /// Başlatıcının pano kipinden gelen eylem; öğe seçilince panel kapanır.
    pub(super) fn clipboard_action(&mut self, cx: &mut Context<Self>, surface: SurfaceId, c: ClipboardCommand) {
        if let Err(e) = self.clipboard_command(c) {
            tracing::debug!("pano eylemi: {e}");
        }
        if let ClipboardCommand::Pick(_) = c {
            self.act(cx, surface, Action::ClosePanel(PanelId::Launcher));
        }
    }
}

impl Shell {
    /// `msg doctor live` pano bölümü (docs/design/doctor.md; yalnız durum, içerik yok):
    /// geçmiş açık mı, compositor data-control sunuyor mu, izleme çalışıyor mu, VM köprüsünün
    /// kipi, kurulu mu ve devrede mi (`auto`'da konuk ajanı bulundu).
    pub(super) fn clipboard_doctor(&self) -> serde_json::Value {
        let mode = self.model.config.clipboard.x11_bridge;
        serde_json::json!({
            "enabled": self.model.config.clipboard.enabled,
            "data_control": self.clipboard.source.is_some(),
            "watching": self.model.clipboard.available,
            "bridge": mode.as_str(),
            "bridge_running": self.clipboard.bridge.is_some(),
            "bridge_active": self.clipboard.bridge_active,
        })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn thumbnails_are_untrusted_and_bounded() {
        let o = super::thumb_options();
        assert_eq!(o.trust(), nuxut_image::Trust::Untrusted, "pano görseli yalnızca yalıtılmış işçide çözülür");
        assert_eq!(o.limits.max_file_bytes, crate::services::clipboard::history::MAX_IMAGE_BYTES as u64);
        assert!(!o.animate);
    }
}
