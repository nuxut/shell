//! nuxut runtime'ına bağlı shell uygulaması.
//!
//! Her output için: tam ekran ana yüzey (`nuxut-shell`, Top, 4 kenar, exclusive -1) ve
//! kalınlığı > 0 olan her kenar için görünmez bir exclusion yüzeyi (Bottom). Yan etkiler
//! (yüzey yarat/yok et, input bölgesi, klavye modu, IPC) yalnızca olay işleyicilerinde;
//! `draw` yapı değiştirmez.
//!
//! Düzen: bu dosya yalnızca çekirdek (yüzeyler, `App` olayları, eylem/mesaj yönlendirme).
//! Her özellik kendi alt modülünde `impl Shell` bloğuyla durur — `start_<x>` (servisi
//! başlatır), `on_<x>` (servis olayı), eylem/timer işleyicileri; buradaki `start`,
//! `act`, `message_inner` ve `timer` yalnızca tek satırlık çağrılarla yönlendirir. Yeni
//! özellik: `app/<x>.rs` + `Shell` alanı + `ShellMessage` varyantı + birer satır çağrı.

mod a11y;
mod calendar;
mod clipboard;
mod clipboard_history;
mod clock;
mod commands;
mod defaults;
mod displays;
mod doctor;
mod hypr_sync;
mod hyprland;
mod idle;
mod input;
mod launcher;
mod night_light;
mod look;
mod magnifier;
mod notify;
mod osd;
mod power;
mod osk;
mod privacy;
mod removable_media;
mod screenshot;
mod session;
mod setup;
mod snapshot;
mod system;
mod sysmon;
mod tray;
mod update_notice;
mod wake;
mod wallpaper;

use std::time::Instant;

use nuxut::{
    Anchor, App, Context, KeyEvent, KeyboardMode, Layer, LayerSurfaceSpec, OutputId, OutputInfo, Painter, PointerEvent,
    Size, SurfaceId, TimerId,
};

use nuxut::theme::AnimatedTheme;

use crate::config::{CONFIG_NAME, ShellConfig};
use crate::ipc;
use crate::layout::Edge;
use crate::model::ShellModel;
use crate::screen::Screen;
use crate::services::calendar::{CalendarEvent, CalendarService};
use crate::services::clock::ClockFormat;
use crate::services::hyprland::{HyprlandClient, HyprlandEvent, HyprlandService};
use crate::services::launcher::{LauncherEvent, LauncherService};
use crate::services::notifications::{NotificationServer, NotifyEvent, NotifyHandle};
use crate::services::session::{SessionEvent, SessionService};
use crate::services::tray::{TrayEvent, TrayHandle, TrayService};
use crate::widgets::Action;

use self::calendar::CalendarQuery;
use self::input::UnfocusTimers;
use self::launcher::LauncherIcons;

pub const MAIN_NAMESPACE: &str = "nuxut";
pub const EXCLUSION_NAMESPACE: &str = "nuxut-exclusion";

/// Arka plan servislerinden ana döngüye gelen mesajlar (`MessageSender` ile).
#[derive(Debug)]
pub enum ShellMessage {
    Hyprland(HyprlandEvent),
    Notify(NotifyEvent),
    Tray(TrayEvent),
    Session(SessionEvent),
    Calendar(CalendarEvent),
    /// Duvar kağıdı çözümü bitti: o monitörün duvar kağıdı yüzeyine kare iste.
    WallpaperReady(OutputId),
    System(crate::system::SystemEvent),
    Launcher(LauncherEvent),
    /// Başlatıcı ikonu arka planda çözüldü (bulunamadıysa `None`).
    LauncherIcon { name: String, image: Option<nuxut_image::Image> },
    /// Masaüstü ayar eşitlemesi turu bitti.
    HyprSync(crate::services::hypr_config::sync::SyncStatus),
    /// Büyüteç görünümü (`app/magnifier.rs` thread'i).
    Magnifier(crate::services::magnifier::MagnifierView),
    /// Shell'in kendi bildirimi (arka plan thread'inden; bkz. `services::notifications::internal`).
    Notice(crate::services::notifications::InternalNotice),
    /// Ekranlar kalıcı yazdı (ya da açılış): üretilen monitör dosyası Nuxut modülüne bağlansın.
    DisplaysPersisted,
    /// Sistem izleyici ölçümü.
    Sysmon(crate::services::sysmon::SysmonEvent),
    /// `msg snapshot restore` bir şey yazdı (ayar, geri yüklenen snapshot).
    SnapshotRestored(nuxut_apply::SettingId, nuxut_apply::SnapshotId, Box<nuxut_apply::RestoredValues>),
    /// Birleşik görünüm turu bitti (rapor + `hypr_config` imleç ortamı).
    Look(crate::services::look::SyncResult),
    /// Kurulu ikon/imleç/GTK temaları değişti (yeni `themes_rev`).
    ThemesChanged(u64),
    /// Boşta kalma: logind olayı (`services::idle::logind`).
    IdleLogind(crate::services::idle::LogindEvent),
    /// Ekran görüntüsü arka plan işi (`app/screenshot.rs`).
    Screenshot(crate::services::screenshot::ShotEvent),
    /// Pano geçmişi (`app/clipboard_history.rs`).
    Clipboard(crate::services::clipboard::ClipboardEvent),
    /// VM pano köprüsü (`app/clipboard_history.rs`).
    ClipboardBridge(crate::services::clipboard::x11::BridgeEvent),
    /// Saat dilimi değişti (`services::timezone`, `app/clock.rs`).
    Timezone(crate::services::timezone::TimezoneEvent),
    /// Çıkarılabilir medya (`services::removable_media`, `app/removable_media.rs`).
    RemovableMedia(crate::services::removable_media::MediaEvent),
    /// Kamera kullanımı (`services::camera`, `app/privacy.rs`).
    Camera(crate::services::camera::CameraEvent),
    /// `msg defaults open <rol>` çözüldü: başlatıcı servisiyle başlat (`app/defaults.rs`).
    LaunchDefault(std::sync::Arc<crate::services::launcher::DesktopEntry>),
    /// Kendi başlattığımız kilit ekranı çıktı (`services::lock_launcher`).
    LockExited(crate::services::lock_launcher::LockExit),
    /// `Session.Lock` sonrası sinyal bekleme süresi doldu.
    LockSignalTimeout,
    /// Oturumu kapat thread'i (`services::logout`).
    Logout(crate::services::logout::LogoutEvent),
}

#[derive(Debug)]
pub struct Shell {
    model: ShellModel,
    clock: ClockFormat,
    clock_timer: Option<TimerId>,
    /// Saat dilimi değişiminden sonra yeniden hesaplama ([`crate::services::timezone::SETTLE`]).
    tz_timer: Option<TimerId>,
    /// timedate1 dinleyicisi (`app/clock.rs`).
    timezone: Option<crate::services::timezone::TimezoneWatch>,
    /// Panel açılınca odağın gelmesi için beklenen süre doldu mu kontrolü.
    grab_timer: Option<TimerId>,
    /// Çizim input bölgesini eskitti (popup/menü boyutu): çizimden hemen sonra eşleme.
    input_timer: Option<TimerId>,
    /// Odağını kaybeden ana yüzeyler; timer dolunca overlay/yaprak kapanır.
    unfocus_timers: UnfocusTimers<SurfaceId, TimerId>,
    /// Bildirim sunucusuna son bildirilen ölçek (`f32::to_bits`).
    notify_scale: Option<u32>,
    /// Bir sonraki popup süre dolumu (`NotificationCenter::next_deadline`).
    notify_timer: Option<TimerId>,
    hypr: Option<HyprlandService>,
    client: Option<HyprlandClient>,
    notify: Option<(NotificationServer, NotifyHandle)>,
    tray: Option<(TrayService, TrayHandle)>,
    session: Option<SessionService>,
    /// Kilit ekranı başlatıcı (`nuxut-lock` çocuğu; `app/session.rs`).
    lock: crate::services::lock_launcher::LockLauncher,
    /// `Session.Lock` gönderildi, logind `Lock` sinyali bekleniyor (gelmezse bekçi doğrudan açar).
    lock_signal_pending: bool,
    calendar: Option<CalendarService>,
    /// Takvim sorgusu: son gönderilen birleşik aralık ve son ay değiştiren görünümün aralığı
    /// (`app/calendar.rs`, `calendar_view::query_range`).
    calendar_query: CalendarQuery,
    system: Option<crate::system::SystemServices>,
    /// Yanıt bekleyen `msg power charge-limit on|off` istekleri (istek kimliği → yanıt;
    /// `app/power.rs`).
    charge_replies: Vec<(u64, nuxut::ipc::Reply)>,
    /// Sistem izleyici (yalnızca bar'da widget'ı varken).
    sysmon: Option<crate::services::sysmon::SysmonService>,
    /// Kamera izleyicisi (yalnızca gizlilik widget'ı kamerayı gösterirken).
    camera: Option<crate::services::camera::CameraService>,
    /// Süren pano okuması ve isteyen yüzey.
    paste_request: Option<(nuxut::wayland::PasteRequestId, SurfaceId)>,
    launcher: Option<LauncherService>,
    launcher_icons: LauncherIcons,
    /// xdg-activation belirteci bekleyen başlatmalar.
    pending_launches: launcher::PendingLaunches<nuxut::ActivationRequestId>,
    /// Belirteç zaman aşımı timer'ı.
    launch_timer: Option<TimerId>,
    /// Renk geçişi için tema (ilk `theme_changed`'den sonra; o zamana kadar `cx.theme()`).
    theme: Option<AnimatedTheme>,
    screens: Vec<Screen>,
    /// Ana yüzeylerin ölçeği (kesirli ölçek `surface_resized` ile gelir): tepsi ikonlarının
    /// çözüleceği fiziksel boyut en büyüğüne göre seçilir.
    main_scales: Vec<(SurfaceId, f32)>,
    /// Tepsi servisine son bildirilen ikon boyutu (fiziksel piksel).
    tray_icon_px: Option<u32>,
    /// OSD görünürlük kararı (kaynak başına son değer).
    osd: osd::OsdTracker,
    osd_timer: Option<TimerId>,
    /// Ekranların uyanma isteği (`Ui::wake_at`) için TEK timer: (timer, en erken an).
    wake_timer: Option<(TimerId, Instant)>,
    /// `--output`'lu son ses/parlaklık komutunun OSD monitörü ve zamanı.
    osd_request: Option<(osd::OsdKind, String, Instant)>,
    /// Bar uç eylemi `workspace` kaydırmasının bekleyen hedefi (widget kaydırmasıyla aynı yol).
    workspace_scroller: crate::widgets::workspaces::page::WorkspaceScroller,
    /// Masaüstü (Hyprland) ayar eşitleyicisi; kapalıysa `None`.
    hypr_sync: Option<hypr_sync::HyprSync>,
    /// Son gönderilen eşitleme isteği (aynısı tekrar gönderilmez).
    hypr_last: Option<crate::services::hypr_config::sync::SyncRequest>,
    /// `msg hypr status` yanıtı (JSON).
    hypr_status: String,
    /// `msg setup status` için son eşitlemenin `source` ve `launch` alanları.
    hypr_brief: (&'static str, Option<&'static str>),
    /// Son eşitlemenin override listesi (hyprlang anahtarı, dosya, satır); `msg input status`.
    hypr_overrides: Vec<(String, String, usize)>,
    /// Eşitleme thread'ine gönderilen son işin sıra numarası (`send_hypr_job`).
    hypr_seq_sent: u64,
    /// Thread'in durumunu bildirdiği en büyük iş numarası; `hypr_seq_sent`'e eşitse sırada iş yok.
    hypr_seq_done: u64,
    /// Son bildirilen eşitleme Hyprland'e canlı uygulandı (`SyncStatus::applied`).
    hypr_applied: bool,
    /// Birleşik görünüm (ADR-0004; `app/look.rs`).
    look: look::LookRuntime,
    /// `msg defaults` thread'i (ilk komutta açılır).
    defaults: defaults::DefaultsRuntime,
    /// Koyu/açık kip yazımı (hızlı anahtar; ilk istekte açılan thread).
    color_mode: crate::services::color_mode::ColorModeWriter,
    /// Dosya yazım ortamı (`nuxut-apply`, ADR-0001): main'de bir kez `Env::from_process`;
    /// varsayılanı kapalı (hiçbir şey yazılmaz). Göç, setup.toml, Hyprland ve Ekranlar yazımları,
    /// `msg snapshot` bunu kullanır.
    apply_env: nuxut_apply::Env,
    /// setup.toml yolu ve yazım izni (ilk açılış; `app/setup.rs`).
    setup: crate::services::setup::SetupAccess,
    /// Son shell.toml sürüm göçünün geri al kaydı (ADR-0002).
    migration_undo: crate::config_migrate::MigrationUndo,
    /// Ayar onarımı: geri al kaydı ve bildirim yinelenmesi (`config_heal`).
    config_heal: crate::config_heal::HealState,
    /// shell.toml yazım politikası ve sürümsüz dosya durumu (ADR-0002); yeniden yüklemeler taşır.
    config_load: crate::config::LoadState,
    /// Ekranlar thread'i (Hyprland yoksa `None`).
    displays: Option<displays::DisplaysWorker>,
    /// Boşta kalma (karart, ekranı kapat, kilitle, uyu; `app/idle.rs`).
    idle: idle::IdleState,
    /// Güvenilmeyen görsellerin yalıtılmış çözücüsü (ADR-0007): açılışta tek kez kurulur; tepsi,
    /// bildirim, medya kapağı ve başlatıcı ikonu Loader'ları paylaşır (havuz ve hız sınırı ortak).
    /// Tembel: ilk güvenilmeyen görselde işçi başlar, boşta süreç kalmaz.
    image_sandbox: std::sync::Arc<crate::services::image_sandbox::Sandbox>,
    /// Duvar kağıdının paylaşılan çözücüsü (duvar kağıdı profilli ayrı `Sandbox`): bütün
    /// çıkışların yüzeyleri aynı örneği alır (birleştirme, önbellek). Tembel: boşta thread/işçi yok.
    wallpaper_decoder: std::sync::Arc<nuxut_image::WallpaperDecoder>,
    /// Yerleşik duvar kağıtlarının paylaşılan bellek içi render'ı (`nuxut:<id>`; kare bileşene
    /// `set_image` ile, dosya/işçi yok). İş varken tek thread, boşta yok.
    wallpaper_builtins: std::sync::Arc<nuxut_wallpapers::Renderer>,
    /// Ekran görüntüsü (`app/screenshot.rs`).
    shot: screenshot::ShotRuntime,
    /// Pano geçmişi (`app/clipboard_history.rs`).
    clipboard: clipboard_history::ClipboardRuntime,
    /// Gece ışığı (`app/night_light.rs`).
    night_light: night_light::NightLightRuntime,
    /// Büyüteç (`app/magnifier.rs`).
    magnifier: magnifier::MagnifierRuntime,
    /// Ekran klavyesi (`app/osk.rs`).
    osk: osk::OskRuntime,
    /// Çıkarılabilir medya (`app/removable_media.rs`).
    removable: removable_media::MediaRuntime,
    started: bool,
}

impl Shell {
    pub fn new(config: ShellConfig) -> Self {
        let image_sandbox = crate::services::image_sandbox::new_shared();
        Self {
            clock: clock::tick_format(&config),
            model: ShellModel::new(config),
            clock_timer: None,
            tz_timer: None,
            timezone: None,
            grab_timer: None,
            input_timer: None,
            unfocus_timers: UnfocusTimers::default(),
            notify_scale: None,
            notify_timer: None,
            hypr: None,
            client: None,
            notify: None,
            tray: None,
            session: None,
            lock: crate::services::lock_launcher::LockLauncher::installed(),
            lock_signal_pending: false,
            calendar: None,
            calendar_query: CalendarQuery::default(),
            system: None,
            charge_replies: Vec::new(),
            sysmon: None,
            camera: None,
            paste_request: None,
            launcher: None,
            launcher_icons: LauncherIcons::default().with_sandbox(crate::services::image_sandbox::decoder(&image_sandbox)),
            pending_launches: Default::default(),
            launch_timer: None,
            theme: None,
            screens: Vec::new(),
            main_scales: Vec::new(),
            tray_icon_px: None,
            osd: osd::OsdTracker::default(),
            osd_timer: None,
            wake_timer: None,
            osd_request: None,
            workspace_scroller: Default::default(),
            hypr_sync: None,
            hypr_last: None,
            hypr_status: String::from("{\"source\":\"pending\"}"),
            hypr_brief: ("pending", None),
            hypr_overrides: Vec::new(),
            hypr_seq_sent: 0,
            hypr_seq_done: 0,
            hypr_applied: false,
            look: look::LookRuntime::default(),
            defaults: defaults::DefaultsRuntime::default(),
            color_mode: Default::default(),
            apply_env: nuxut_apply::Env { disabled: true, ..nuxut_apply::Env::default() },
            setup: crate::services::setup::SetupAccess::default(),
            migration_undo: crate::config_migrate::MigrationUndo::default(),
            config_heal: crate::config_heal::HealState::default(),
            config_load: crate::config::LoadState::default(),
            displays: None,
            idle: idle::IdleState::default(),
            image_sandbox,
            wallpaper_decoder: crate::services::image_sandbox::new_wallpaper_decoder(),
            wallpaper_builtins: nuxut_wallpapers::Renderer::new(),
            shot: screenshot::ShotRuntime::default(),
            clipboard: Default::default(),
            night_light: night_light::NightLightRuntime::default(),
            magnifier: magnifier::MagnifierRuntime::default(),
            osk: osk::OskRuntime::default(),
            removable: Default::default(),
            started: false,
        }
    }

    /// İlk output'ta bir kez (run başladıktan sonra: servis thread'leri sinyal maskesini
    /// devralır): config izleme, servisler, saat. Bir servisin başlamaması shell'i
    /// durdurmaz; ilgili özellik boş kalır.
    fn start(&mut self, cx: &mut Context<Self>) {
        if std::mem::replace(&mut self.started, true) {
            return;
        }
        cx.watch_config(CONFIG_NAME);
        self.start_hyprland(cx);
        // Görünüm önce: ilk turu imleç ortamını verir, Hyprland eşitlemesi onu bekler.
        self.start_look(cx);
        self.start_hypr_sync(cx);
        self.start_magnifier(cx);
        self.start_osk(cx);
        self.start_displays(cx);
        self.start_notifications(cx);
        self.start_tray(cx);
        self.start_session(cx);
        self.start_calendar(cx);
        self.start_system(cx);
        self.start_launcher(cx);
        self.sync_sysmon_service(cx);
        self.sync_camera_service(cx);
        self.start_idle(cx);
        self.start_clipboard(cx);
        self.start_timezone(cx);
        self.start_night_light(cx);
        self.start_removable_media(cx);
        // Hoş geldin açıldıysa güncelleme bildirimi bastırılır (ikisi birlikte çıkmaz).
        let welcome = self.offer_welcome();
        self.check_update_notice(cx, welcome);
        self.sync_notify_scale();
        self.tick_clock(cx);
    }

    /// Her ekran metin kuran her şeyi zorla yeniden kurar ([`Screen::relocalize`]): dil/biçim
    /// değişimi ve saat dilimi değişimi (`app/clock.rs`) ortak yolu. Servisler yeniden bağlanmaz,
    /// model aynı kalır, açık yapraklar ve odak korunur.
    fn relocalize_screens(&mut self, cx: &mut Context<Self>) {
        let theme = cx.theme().clone();
        let now = Instant::now();
        for i in 0..self.screens.len() {
            let actions = self.screens[i].relocalize(&self.model, cx.text(), &theme, now);
            let surface = self.screens[i].main;
            for a in actions {
                self.act(cx, surface, a);
            }
            cx.request_frame(surface);
        }
        // Kartlar yeniden kuruldu (bugünün ayına döndü): sorgu görünür takvimlere.
        self.refresh_calendar_range(false);
        // Metin genişlikleri değişti: input bölgesi yeni yerleşime; başlatıcı açıksa yenilenen
        // sonuçların ikonları.
        self.refresh_inputs(cx);
        self.pump_launcher_icons(cx);
    }

    /// Model değişti: her ekrana yansıt, değişen yüzeylere kare iste.
    fn model_changed(&mut self, cx: &mut Context<Self>) {
        let theme = cx.theme().clone();
        let now = Instant::now();
        for s in &mut self.screens {
            if s.sync_model(&self.model, &theme, now) {
                cx.request_frame(s.main);
            }
        }
    }

    /// Exclusion yüzeylerini hedef kalınlıklarla eşler: gerekeni yaratır, 0 olanı kaldırır.
    fn sync_exclusions(cx: &mut Context<Self>, screen: &mut Screen, config: &ShellConfig) {
        let zones = screen.desired_zones(config);
        for (i, edge) in Edge::ALL.into_iter().enumerate() {
            let zone = zones[i];
            match (screen.exclusions[i], zone > 0) {
                (Some((id, current)), true) => {
                    // Kalınlık değişti: yüzey boyutu da zone'a eşit olsun diye yeniden yarat.
                    if current != zone {
                        cx.destroy_surface(id);
                        let id = cx.create_layer_surface(exclusion_spec(screen.output, edge, zone));
                        cx.a11y_exclude(id);
                        cx.set_input_region(id, &[]);
                        screen.exclusions[i] = Some((id, zone));
                    }
                }
                (Some((id, _)), false) => {
                    cx.destroy_surface(id);
                    screen.exclusions[i] = None;
                }
                (None, true) => {
                    let id = cx.create_layer_surface(exclusion_spec(screen.output, edge, zone));
                    cx.a11y_exclude(id);
                    cx.set_input_region(id, &[]);
                    screen.exclusions[i] = Some((id, zone));
                }
                (None, false) => {}
            }
        }
    }

    fn act(&mut self, cx: &mut Context<Self>, surface: SurfaceId, action: Action) {
        match action {
            Action::RefreshInput => {
                self.refresh_inputs(cx);
                cx.request_frame(surface);
            }
            Action::Notify(cmd) => self.on_notify_command(cx, cmd),
            Action::Tray(cmd) => {
                // Tek geçici yüzey: menü açılırken geçici yaprak hemen (menüyü beklemeden,
                // kendi kapanış animasyonuyla) kapanır; sabit panel kalır.
                if cmd.opens_menu()
                    && let Some(open) = self.screens.iter().find(|s| s.main == surface).and_then(|s| s.closed_by_bar_menu())
                {
                    self.act(cx, surface, Action::ClosePanel(open));
                }
                self.tray_command(cmd);
            }
            Action::CalendarRange { from, to } => {
                self.calendar_range_changed(from, to);
                cx.request_frame(surface);
            }
            Action::Session(a) => self.session_action(cx, a),
            Action::RequestPaste => self.request_paste(cx, surface),
            Action::System(cmd) => {
                self.system_command(cx, cmd);
            }
            Action::SwitchWorkspace(id) => self.switch_workspace(id),
            Action::BarEdge(cmd) => self.bar_edge_command(cx, surface, cmd),
            Action::TogglePanel(_) | Action::ClosePanel(_) | Action::PinPanel(..) => self.panel_action(cx, surface, action),
            Action::Launch { entry, action } => self.launch_app(cx, surface, entry, action),
            Action::Copy(text) => self.copy_to_clipboard(cx, text.as_str()),
            Action::Clipboard(c) => self.clipboard_action(cx, surface, c),
            Action::PickEmoji(e) => self.pick_emoji(cx, surface, e),
            Action::NightLight(cmd) => {
                if let Err(e) = self.night_light_command(cx, cmd) {
                    tracing::warn!("gece ışığı: {e}");
                }
            }
            Action::Magnifier(cmd) => {
                if let Err(e) = self.magnifier_action(cmd) {
                    tracing::warn!("büyüteç: {e}");
                }
            }
            Action::SetThemeMode(mode) => self.color_mode.request(&self.apply_env, mode),
            Action::IdleInhibit(on) => {
                let t = if on { ipc::Toggle::On } else { ipc::Toggle::Off };
                let _ = self.idle_command(cx, ipc::IdleCommand::Inhibit(t));
            }
            Action::KeyboardLayoutNext => self.keyboard_layout_next(),
            Action::Osk(cmd) => {
                if let Err(e) = self.osk_action(cx, cmd, None) {
                    tracing::warn!("ekran klavyesi: {e}");
                }
            }
        }
    }

    /// Yaprak aç/kapa ve açılış kancaları.
    fn panel_action(&mut self, cx: &mut Context<Self>, surface: SurfaceId, action: Action) {
        let now = Instant::now();
        let theme = cx.theme().clone();
        let config = self.model.config.clone();
        let Some(idx) = self.screens.iter().position(|s| s.main == surface) else { return };
        let transition = {
            let text = cx.text();
            let s = &mut self.screens[idx];
            match action {
                Action::TogglePanel(p) => s.toggle_panel(p, text, &config, &theme, now),
                Action::ClosePanel(p) => s.close_panel(p, text, &config, &theme, now),
                Action::PinPanel(e, on) => s.set_pinned(e, on, text, &config, &theme, now),
                _ => return,
            }
        };
        if self.screens[idx].transient_panel().is_some_and(|p| transition.opened.contains(&p)) {
            self.restart_focus_grab(cx);
        }
        self.refresh_inputs(cx);
        let opened_any = !transition.opened.is_empty();
        // Çekmece açılış kancaları (bildirim merkezi → okundu).
        for p in transition.opened {
            for a in self.screens[idx].panel_opened(p, now) {
                self.act(cx, surface, a);
            }
        }
        // Açılan/kapanan yaprak görünür takvimleri değiştirir (popout, kartlı panel).
        self.refresh_calendar_range(opened_any);
        if opened_any {
            self.magnifier_refresh();
        }
        self.sync_sysmon_detail();
        self.pump_launcher_icons(cx);
        cx.request_frame(surface);
    }

    fn message_inner(&mut self, cx: &mut Context<Self>, message: ShellMessage) {
        match message {
            ShellMessage::Hyprland(e) => self.on_hyprland(cx, e),
            ShellMessage::Notify(e) => self.on_notify(cx, e),
            ShellMessage::Tray(e) => self.on_tray(cx, e),
            ShellMessage::Session(e) => self.on_session(cx, e),
            ShellMessage::LockExited(e) => self.on_lock_exited(cx, e),
            ShellMessage::LockSignalTimeout => self.on_lock_signal_timeout(cx),
            ShellMessage::Logout(e) => self.on_logout(e),
            ShellMessage::Calendar(e) => self.on_calendar(cx, e),
            ShellMessage::WallpaperReady(output) => self.on_wallpaper_ready(cx, output),
            ShellMessage::System(e) => self.on_system(cx, e),
            ShellMessage::Launcher(e) => self.on_launcher(cx, e),
            ShellMessage::LaunchDefault(entry) => self.launch_default(entry),
            ShellMessage::LauncherIcon { name, image } => self.on_launcher_icon(cx, name, image),
            ShellMessage::HyprSync(s) => self.on_hypr_sync(s),
            ShellMessage::Magnifier(v) => self.on_magnifier(cx, v),
            ShellMessage::Look(r) => self.on_look(cx, r),
            ShellMessage::ThemesChanged(_) => self.themes_changed(cx),
            ShellMessage::Notice(n) => self.notice(cx, n),
            ShellMessage::DisplaysPersisted => self.force_hypr_sync(cx),
            ShellMessage::Sysmon(e) => self.on_sysmon(cx, e),
            ShellMessage::SnapshotRestored(setting, id, values) => self.on_snapshot_restored(cx, setting, id, values),
            ShellMessage::IdleLogind(e) => self.on_idle_logind(cx, e),
            ShellMessage::Screenshot(e) => self.on_shot(cx, e),
            ShellMessage::Clipboard(e) => self.on_clipboard(cx, e),
            ShellMessage::ClipboardBridge(e) => self.on_clipboard_bridge(cx, e),
            ShellMessage::Timezone(e) => self.on_timezone(cx, e),
            ShellMessage::RemovableMedia(e) => self.on_removable_media(cx, e),
            ShellMessage::Camera(e) => self.on_camera(cx, e),
        }
        // Kilit ve etkin pencere her türlü servis olayıyla değişebilir (Hyprland, logind).
        self.sync_clipboard_gate();
    }
}

fn exclusion_spec(output: OutputId, edge: Edge, zone: i32) -> LayerSurfaceSpec {
    // Yalnızca zone > 0 için çağrılır: runtime 0 kalınlığı 1 px'e yükseltir (1 px itme olur).
    let t = zone.max(1) as u32;
    let (anchor, size) = match edge {
        Edge::Top => (Anchor::TOP | Anchor::LEFT | Anchor::RIGHT, (0, t)),
        Edge::Bottom => (Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT, (0, t)),
        Edge::Left => (Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM, (t, 0)),
        Edge::Right => (Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM, (t, 0)),
    };
    LayerSurfaceSpec {
        layer: Layer::Bottom,
        anchor,
        size,
        exclusive_zone: zone,
        keyboard: KeyboardMode::None,
        ..LayerSurfaceSpec::new(EXCLUSION_NAMESPACE, output)
    }
}

impl App for Shell {
    type Message = ShellMessage;

    fn output_added(&mut self, cx: &mut Context<Self>, output: &OutputInfo) {
        let spec = LayerSurfaceSpec {
            layer: Layer::Top,
            anchor: Anchor::ALL,
            exclusive_zone: -1,
            keyboard: KeyboardMode::None,
            ..LayerSurfaceSpec::new(MAIN_NAMESPACE, output.id)
        };
        let main = cx.create_layer_surface(spec);
        let theme = cx.theme().clone();
        let mut screen = Screen::new(&theme, &self.model.config, output.id, output.name.clone(), main);
        screen.focus_grab_extra = self.osk_surface();
        screen.set_size(output.size);
        screen.position = output.position;
        screen.sync_model(&self.model, &theme, Instant::now());
        Self::sync_exclusions(cx, &mut screen, &self.model.config);
        tracing::info!(output = %output.name, size = ?(output.size.width, output.size.height), "output eklendi");
        self.screens.push(screen);
        self.shot_output_changed(output);
        self.night_light_output_added(cx, output.id);
        let idx = self.screens.len() - 1;
        self.sync_wallpaper(cx, idx);
        self.start(cx);
    }

    fn output_changed(&mut self, _cx: &mut Context<Self>, output: &OutputInfo) {
        if let Some(s) = self.screens.iter_mut().find(|s| s.output == output.id) {
            s.name.clone_from(&output.name);
            s.position = output.position;
        }
        self.shot_output_changed(output);
    }

    fn output_removed(&mut self, cx: &mut Context<Self>, output: OutputId) {
        self.screens.retain(|s| {
            if s.output != output {
                return true;
            }
            if let Some(id) = s.focus_grab {
                cx.focus_grab_release(id);
            }
            cx.destroy_surface(s.main);
            for (id, _) in s.exclusions.iter().flatten() {
                cx.destroy_surface(*id);
            }
            if let Some(wp) = &s.wallpaper {
                wallpaper::destroy_wallpaper(cx, wp);
            }
            false
        });
        self.idle_output_removed(cx, output);
        self.night_light_output_removed(output);
        self.shot_output_removed(cx, output);
        self.osk_output_removed(cx, output);
        let screens = &self.screens;
        self.main_scales.retain(|(id, _)| screens.iter().any(|s| s.main == *id));
        self.sync_tray_icon_size();
        self.sync_notify_scale();
    }

    fn surface_resized(&mut self, cx: &mut Context<Self>, surface: SurfaceId, size: Size, scale: f32) {
        if self.wallpaper_resized(cx, surface, size, scale) || self.shot_resized(cx, surface, size) || self.osk_resized(cx, surface, size) {
            return;
        }
        let config = self.model.config.clone();
        let Some(idx) = self.screens.iter().position(|s| s.main == surface) else { return };
        let s = &mut self.screens[idx];
        s.set_size(size);
        let output = s.output;
        Self::sync_exclusions(cx, &mut self.screens[idx], &config);
        Self::apply_input(cx, &mut self.screens[idx], &config);
        cx.request_frame(surface);
        self.osk_screen_resized(cx, output);
        match self.main_scales.iter_mut().find(|(id, _)| *id == surface) {
            Some((_, s)) => *s = scale,
            None => self.main_scales.push((surface, scale)),
        }
        self.sync_tray_icon_size();
        self.sync_notify_scale();
    }

    fn pointer(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: PointerEvent) {
        if self.shot_pointer(cx, surface, event) || self.osk_pointer(cx, surface, event) {
            return;
        }
        let theme = cx.theme().clone();
        let config = self.model.config.clone();
        let now = Instant::now();
        let text = cx.text();
        let Some(s) = self.screens.iter_mut().find(|s| s.main == surface) else { return };
        let (redraw, action) = s.pointer(text, &config, &theme, event, now);
        // Runtime aynı şekli tekrar göndermez; her olayda çağırmak ucuz.
        let cursor = s.cursor();
        cx.set_cursor(surface, cursor);
        if redraw {
            cx.request_frame(surface);
        }
        // Hover ipucu gecikmesini başlatmış olabilir.
        self.rearm_wake(cx);
        if let Some(a) = action {
            self.act(cx, surface, a);
        }
        self.pump_launcher_icons(cx);
        self.refresh_inputs(cx);
    }

    fn keyboard(&mut self, cx: &mut Context<Self>, surface: SurfaceId, event: KeyEvent) {
        if self.shot_key(cx, surface, &event) {
            return;
        }
        self.keyboard_inner(cx, surface, event);
        self.pump_launcher_icons(cx);
        self.refresh_inputs(cx);
    }

    fn keyboard_focus(&mut self, cx: &mut Context<Self>, surface: SurfaceId, focused: bool) {
        if self.shot_owns(surface) {
            return;
        }
        self.keyboard_focus_inner(cx, surface, focused);
        self.refresh_inputs(cx);
    }

    fn focus_grab_cleared(&mut self, cx: &mut Context<Self>, id: nuxut::FocusGrabId) {
        self.focus_grab_cleared_inner(cx, id);
        self.refresh_inputs(cx);
    }

    fn message(&mut self, cx: &mut Context<Self>, message: ShellMessage) {
        self.message_inner(cx, message);
        self.refresh_inputs(cx);
    }

    fn paste(
        &mut self,
        cx: &mut Context<Self>,
        id: nuxut::wayland::PasteRequestId,
        result: Result<nuxut::clipboard::ClipboardText, nuxut::clipboard::PasteError>,
    ) {
        self.on_paste(cx, id, result);
        self.refresh_inputs(cx);
    }

    /// `nuxut-shell msg …` komutları. Satır asla shell'e verilmez; sabit sözdizimiyle
    /// ayrıştırılır (`ipc::parse_args`), hata `error: …` ile döner.
    fn command(&mut self, cx: &mut Context<Self>, args: Vec<String>, reply: nuxut::ipc::Reply) {
        let parsed = ipc::parse_args(&args);
        // Yanıtı ses servisi (ayrı thread) verir; UI thread'i beklemez.
        if let Ok(ipc::Command::AudioDiagnose) = parsed {
            return self.audio_diagnose(reply);
        }
        // `nuxut-shell doctor`un canlı bölümü: ses özeti ses thread'inden.
        if let Ok(ipc::Command::DoctorLive) = parsed {
            return self.doctor_live(cx, reply);
        }
        // Ekranlar: yanıtı ekran thread'i verir (Hyprland istekleri UI'ı bekletmez).
        let parsed = match parsed {
            Ok(ipc::Command::Displays(c)) => return self.displays_command(c, reply),
            // Girdi durumu: Hyprland `getoption` istekleri ayrı thread'de.
            Ok(ipc::Command::InputStatus) => return self.input_status_command(reply),
            // Büyüteç: Hyprland istekleri büyüteç thread'inde.
            Ok(ipc::Command::Magnifier(c)) => return self.magnifier_command(c, reply),
            // Varsayılan uygulamalar: dizin taraması ve snapshot kilidi ayrı thread'de.
            Ok(ipc::Command::Defaults(c)) => return self.defaults_command(cx, c, reply),
            // Snapshot deposu kilidi beklenebilir: yanıtı komut thread'i verir.
            Ok(ipc::Command::Snapshot(c)) => return self.snapshot_command(cx, c, reply),
            // Ekran görüntüsü: yanıt iş (ve bölge seçimi) bitince.
            Ok(ipc::Command::Shot(c)) => return self.shot_command(cx, c, Some(reply)),
            // Şarj sınırı: yanıt UPower cevap verince (`app/power.rs`).
            Ok(ipc::Command::Power(ipc::PowerCommand::SetChargeLimit(on))) => return self.charge_limit_command(on, reply),
            other => other,
        };
        let result = parsed.and_then(|c| self.execute(cx, c));
        self.refresh_inputs(cx);
        match result {
            Ok(text) => reply.send(&text),
            Err(e) => reply.send(&format!("error: {e}")),
        }
    }

    fn idle(&mut self, cx: &mut Context<Self>, id: nuxut::IdleWatchId, idle: bool) {
        self.on_idle_watch(cx, id, idle);
    }

    /// Pano izleme (`app/clipboard_history.rs`). Okuma servis thread'inde tutamaçla yapılır;
    /// `data_control_read` kullanılmaz.
    fn data_control(&mut self, cx: &mut Context<Self>, event: nuxut::DataControlEvent) {
        self.on_data_control(cx, event);
    }

    fn color_transform_blocked(&mut self, cx: &mut Context<Self>) {
        self.night_light_blocked(cx);
    }

    /// SIGTERM/SIGINT/SIGHUP: gece ışığı açıkça bırakılır (framework kapanışı da bırakır), sonra
    /// varsayılan gibi çıkış.
    fn terminate(&mut self, cx: &mut Context<Self>) {
        tracing::info!("sinyal: kapanılıyor");
        self.night_light_shutdown(cx);
        self.magnifier_shutdown();
        self.osk_shutdown(cx);
        cx.quit();
    }

    fn timer(&mut self, cx: &mut Context<Self>, id: TimerId) {
        // Girdi eşlemesi gerektirmeyen timer'lar erken döner.
        if self.notify_timer_fired(cx, id)
            || self.osd_timer_fired(cx, id)
            || self.launch_timer_fired(cx, id)
            || self.wake_timer_fired(cx, id)
            || self.idle_timer_fired(cx, id)
            || self.night_light_timer_fired(cx, id)
            || self.clipboard_timer_fired(cx, id)
        {
            return;
        }
        let _ = self.input_timer_fired(cx, id) || self.clock_timer_fired(cx, id);
        self.refresh_inputs(cx);
    }

    fn activation_token(
        &mut self,
        _cx: &mut Context<Self>,
        id: nuxut::ActivationRequestId,
        token: Option<nuxut::ActivationToken>,
    ) {
        self.on_activation_token(id, token);
    }

    fn keyboard_layout_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(name) = cx.keyboard_layout().map(str::to_owned) {
            self.osk_compositor_layout(cx, &name);
        }
    }

    fn theme_changed(&mut self, cx: &mut Context<Self>) {
        let theme = cx.theme().clone();
        let now = Instant::now();
        // Renkler eski temadan yenisine `motion.effects` ile akar (DESIGN: tema değişimi
        // animasyonludur); token'lar (yay, yarıçap, tipografi) hemen geçer.
        match &mut self.theme {
            Some(a) => a.set(theme.clone(), now),
            None => self.theme = Some(AnimatedTheme::new(theme.clone())),
        }
        for s in &mut self.screens {
            s.set_theme(&self.model.config, &theme, now);
            cx.request_frame(s.main);
            if let Some(wp) = &mut s.wallpaper
                && wp.set_theme(&theme, now)
            {
                cx.request_frame(wp.surface);
            }
        }
        // GTK/Qt/ikon/imleç (birleşik görünüm; açıksa).
        self.request_look_sync(cx);
        // Köşe, gölge, hareket, renkler pencerelere de.
        self.request_hypr_sync(cx);
        self.osk_theme_changed(cx);
    }

    /// Dil değişti (ADR-0010): süreç geneli katalog zaten yeni. Her ekran metin kuran her şeyi
    /// zorla yeniden kurar ([`Screen::relocalize`]); servisler yeniden bağlanmaz, model aynı
    /// kalır, açık yapraklar ve odak korunur.
    fn locale_changed(&mut self, cx: &mut Context<Self>) {
        self.relocalize_screens(cx);
        // `[locale] hour_cycle`/`formats` saat biçimini değiştirmiş olabilir (t119): timer'ın
        // biçimi yeni locale'le, saat hemen yeniden çizilir.
        self.clock = clock::tick_format(&self.model.config);
        self.tick_clock(cx);
        tracing::info!("dil/biçim değişti: ekranlar yeniden kuruldu");
    }

    fn config_changed(&mut self, cx: &mut Context<Self>, name: &str) {
        if name == CONFIG_NAME {
            self.reload_config(cx);
        }
    }

    fn accessibility(&mut self, cx: &mut Context<Self>, surface: SurfaceId, tree: &mut nuxut::access::AccessTree) {
        self.accessibility_inner(cx, surface, tree);
    }

    fn accessibility_action(&mut self, cx: &mut Context<Self>, surface: SurfaceId, request: nuxut::access::AccessRequest) {
        self.accessibility_action_inner(cx, surface, request);
    }

    fn draw(&mut self, cx: &mut Context<Self>, surface: SurfaceId, now: Instant, painter: &mut Painter) {
        if self.draw_wallpaper(cx, surface, now, painter) {
            return;
        }
        if self.draw_idle(cx, surface, now, painter) || self.draw_shot(cx, surface, now, painter) || self.draw_osk(cx, surface, now, painter) {
            return;
        }
        // Renk geçişi sürüyorsa karenin renkleri ara değerdir; tüm yüzey yeniden çizilir.
        let colors_moving = self.theme.as_mut().is_some_and(|a| a.is_animating(now));
        let theme = match &self.theme {
            Some(a) if colors_moving => a.current(now),
            _ => cx.theme().clone(),
        };
        let Some(s) = self.screens.iter_mut().find(|s| s.main == surface) else {
            return; // exclusion yüzeyleri şeffaf kalır
        };
        if colors_moving {
            s.invalidate();
        }
        let redraw = s.draw(painter, &self.model.config, &theme, now);
        if redraw || colors_moving {
            cx.request_frame(surface);
        }
        // Popup yığını/menü boyutunu bu karede öğrendi: input bölgesi `draw` içinde
        // değiştirilemez, hemen sonraki tek atımlık timer'da eşlenir (bir kare gecikme).
        if self.input_timer.is_none() && s.input_stale(&self.model.config) {
            self.input_timer = Some(cx.set_timer(std::time::Duration::ZERO));
        }
        // Karenin `Ui::wake_deadline`'ları ekranda toplandı: tek timer'a.
        self.rearm_wake(cx);
    }
}
