//! Shell komut modeli: `nuxut-shell msg <komut…>` satırları ve CLI argümanları.
//!
//! Komutlar runtime'ın tek örnek IPC'si (`$XDG_RUNTIME_DIR/nuxut/shell.sock`, satır başına
//! tek istek) üzerinden gelir; burada yalnızca **saf ayrıştırma** var. Satır asla bir shell'e
//! verilmez: sabit bir sözdizimine göre tiplere çevrilir, gerisi reddedilir. Yanıt kuralı:
//! başarı `ok` (ya da veri), hata `error: …`.
//!
//! ```text
//! panel <toggle|open|close> <right|left|top|bottom|clock|launcher|sysmon> [--output <ad>]
//! launcher <toggle|open|close|clipboard|emoji> [--output <ad>]
//! clipboard list | clipboard pick <id> | clipboard delete <id> | clipboard clear
//! panel close-all [--output <ad>]
//! bar <toggle|show|hide> [--output <ad>]
//! volume <up|down> [<adım%>] | volume set <0-150> | volume mute [toggle|on|off] [--output <ad>]
//! mic mute [toggle|on|off] [--output <ad>]
//! brightness <up|down> [<adım%>] | brightness set <0-100> [--output <ad>]
//! config reload | config status | config undo | config undo-migration | config heal [--dry-run|--undo|--fix-typos|--ignore-typos]
//! audio-diagnose
//! doctor live   (iç: `nuxut-shell doctor` kullanır; kullanıcı `nuxut-shell doctor` çalıştırır)
//! hypr <status|sync|add-source|undo|apply-switch>
//! displays <list|status> | displays apply <JSON> | displays <confirm|revert> <jeton>
//! snapshot list [--setting <ayar>] [--limit <1-200>] | snapshot show <kimlik>
//! snapshot restore <kimlik> [--force] [--setting <ayar>]
//! setup status | setup <done|skip> <1-65535>
//! look <status|plan|use-other-tool> | look restart-app <uygulama>
//! idle <status|inhibit|uninhibit|toggle>
//! lock
//! night-light <status|on|off|toggle> | night-light temperature <1500-6000>
//! magnifier <status|in|out|reset|on|off|toggle> | magnifier set <1-10>
//! osk <toggle|show|hide> [--output <ad>] | osk status
//! images status
//! wallpaper builtins
//! shot region|window|output|screen [--output <ad>] [--no-save] [--no-copy] | shot open|reveal <n>
//! input status
//! clock refresh
//! defaults status | defaults set <rol> <masaüstü-kimliği>
//! removable list | removable open <id> | removable eject <id>
//! power status | power charge-limit <on|off|status>
//! bluetooth agent-status
//! ping | version | quit | help
//! ```
//! `--output` verilmezse panel komutları odaklı monitöre, bar komutları tüm monitörlere uygulanır;
//! ses/parlaklık komutlarında `--output` yalnızca OSD'nin gösterileceği monitördür.

use crate::layout::Edge;
use crate::widgets::PanelId;

/// Bir komut satırının en çok uzunluğu (runtime 4 KiB'ı zaten sınırlar).
pub const MAX_LINE: usize = 512;
const MAX_TOKENS: usize = 8;
/// Monitör adı sınırı (Hyprland bağlayıcı adları kısa: `DP-1`, `HDMI-A-2`…).
const MAX_OUTPUT_NAME: usize = 64;

/// `config heal` alt komutu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealCommand {
    /// Onar ve yaz (önce yedek).
    Apply,
    /// Yalnızca ne yapılacağını yaz.
    DryRun,
    /// Son onarımı geri al; onarımın sildiği anahtarlar korunur.
    Undo,
    /// Yazım hatası önerilerini uygula.
    FixTypos,
    /// Önerileri yok say (koru listesine).
    IgnoreTypos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toggle {
    Toggle,
    On,
    Off,
}

/// Ses/parlaklık düzeyi değişimi (yüzde). Sınırlar (ses tavanı, parlaklık tabanı) uygulamada
/// kırpılır, hata değildir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Up(u8),
    Down(u8),
    Set(u8),
}

/// `up`/`down` varsayılan adımı (%).
pub const DEFAULT_STEP: u8 = 5;
/// `volume set` üst sınırı: `[audio] max_volume`'ün izin verdiği en yüksek değer.
pub const MAX_VOLUME_SET: u8 = 150;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Panel { panel: PanelId, action: Toggle, output: Option<String> },
    /// Başlatıcıyı bir kipte aç (`launcher clipboard`, `launcher emoji`); o kipte açıksa kapat,
    /// başka kipte açıksa kipi değiştir.
    LauncherMode { mode: crate::launcher_view::LauncherMode, output: Option<String> },
    /// Oturum eylemlerinin olduğu kenar panelini aç (`session menu`; CTRL+ALT+Delete).
    SessionMenu { output: Option<String> },
    /// Pano geçmişi (`services::clipboard`).
    Clipboard(crate::services::clipboard::ClipboardCommand),
    ClosePanels { output: Option<String> },
    /// Kenar panelini sabitle (`On`: kapalıysa açılır) / sabitlemeyi kaldır.
    PinPanel { edge: Edge, action: Toggle, output: Option<String> },
    /// `On` = bar görünür.
    Bar { action: Toggle, output: Option<String> },
    /// Bar'da klavye gezinmesi (ekran okuyucu ve yalnız klavye için; Escape çıkar).
    BarFocus { output: Option<String> },
    /// `output`: OSD'nin gösterileceği monitör (yoksa odaklı).
    Volume { level: Level, output: Option<String> },
    /// `On` = sessiz.
    VolumeMute { action: Toggle, output: Option<String> },
    /// `On` = mikrofon sessiz.
    MicMute { action: Toggle, output: Option<String> },
    Brightness { level: Level, output: Option<String> },
    ReloadConfig,
    /// Son shell.toml sürüm göçünün kullanıcı değerlerini geri koyar (ADR-0002 "Geri al").
    UndoMigration,
    /// Ayar dosyalarının kendini onarması (`config_heal`, docs/design/config-migrations.md).
    ConfigHeal(HealCommand),
    /// Ayar dosyalarının durumu (JSON): sürüm, bekleyen onarım, geri al, yedekler.
    ConfigStatus,
    /// Son ayar değişikliğini geri alır: önce onarım, yoksa sürüm göçü.
    ConfigUndo,
    /// Ses tanı raporu (seviye yolu, route/düğüm ham alanları, `wpctl get-volume`); yanıt
    /// ses servisinden gelir (bkz. `services::audio`).
    AudioDiagnose,
    /// `nuxut-shell doctor`un canlı bölümü: tek satır JSON (pid, sürüm, örnek, hypr/look durumu,
    /// ses özeti; docs/design/doctor.md). Yanıtı ses thread'i verir.
    DoctorLive,
    /// Masaüstü (Hyprland) ortaklığı.
    Hypr(HyprCommand),
    /// Ekranlar (ADR-0005 §7); yanıtı ekran thread'i verir (`app/displays.rs`).
    Displays(DisplaysCommand),
    /// Snapshot listele/göster/geri yükle (ADR-0001 §9.3); yanıtı ayrı thread verir.
    Snapshot(SnapshotCommand),
    /// İlk açılış / Hoş geldin durumu (ADR-0001 §10; `services::setup`).
    Setup(SetupCommand),
    /// Birleşik görünüm (ADR-0004): durum ve plan (tek satır JSON).
    Look(LookCommand),
    /// Boşta kalma: durum (tek satır JSON) ya da kahve modu (`On` = boşta eylemleri durur).
    Idle(IdleCommand),
    /// Gece ışığı (`services::night_light`): durum (tek satır JSON) ya da elle aç/kapa/sıcaklık.
    NightLight(NightLightCommand),
    /// Büyüteç (`services::magnifier`, Hyprland `cursor:zoom_factor`): durum (tek satır JSON) ya
    /// da büyüt/küçült/sıfırla/çarpan/aç/kapa. Yanıtı büyüteç thread'i verir (`app/magnifier.rs`).
    Magnifier(MagnifierCommand),
    /// Ekran klavyesi (`services::osk`): göster/gizle (hedef çıkış: `--output` ya da odaklı
    /// monitör) ya da durum (tek satır JSON).
    Osk { action: OskAction, output: Option<String> },
    /// Görsel yalıtımı (framework ADR-0007): yalıtım kipi ve sayaçlar (tek satır JSON; içerik
    /// taşımaz).
    ImagesStatus,
    /// Yerleşik duvar kağıtları (ayarlar seçicisi; `docs/design/wallpapers.md`): tek satır JSON dizi
    /// `[{id, source, name, variants, default, seed: {dark, light}}]`, ad shell'in dilinde.
    WallpaperBuiltins,
    /// Ekran görüntüsü (`services::screenshot`); yakalamanın yanıtı iş bitince (`app/screenshot.rs`).
    Shot(crate::services::screenshot::ShotCommand),
    /// Girdi aygıtları (sözleşme input-v1): Hyprland'deki etkin değerler (`getoption`), yönetilen
    /// ve geçersiz kılınan `[input]` anahtarları (tek satır JSON; salt okunur). Yanıtı ayrı
    /// thread verir (`app/hypr_sync.rs`).
    InputStatus,
    /// Saat dilimi değişmiş olabilir: saat, takvim ve saate bağlı her şey [`crate::services::timezone::SETTLE`]
    /// sonra yeniden hesaplanır (Ayarlar `SetTimezone`dan sonra; timedate1 sinyalinin yedeği). Yanıt `ok`.
    ClockRefresh,
    /// Oturumu kilitle: Kilitle düğmesiyle aynı yol (logind `Session.Lock` → `Lock` sinyali →
    /// `nuxut-lock`; logind yoksa doğrudan `nuxut-lock`). SUPER+L buna bağlanır. Yanıt `ok`.
    Lock,
    /// Varsayılan uygulamalar (sözleşme defaults-v1; `services::default_apps`): durum ya da
    /// yazım (tek satır JSON). Yanıtı ayrı thread verir (`app/defaults.rs`).
    Defaults(DefaultsCommand),
    /// Çıkarılabilir medya (`services::removable_media`): birim listesi (tek satır JSON), aç, çıkar.
    /// Kimlik udisks blok adı (`sdb1`); bilinmeyen kimlik hata yanıtı alır.
    Removable(crate::services::removable_media::MediaCommand),
    /// Güç (`services::power`): durum (tek satır JSON) ya da pil şarj sınırı aç/kapa (yanıtı
    /// UPower cevap verince gelir; `app/power.rs`).
    Power(PowerCommand),
    /// Kabuğun Bluetooth eşleştirme ajanı durumu, tek satır JSON
    /// `{"registered":bool,"default":bool}` (Ayarlar `Pair` öncesi sorar; servis yoksa ya da
    /// eşleştirme ajansız örnekte ikisi `false`). Yanıt ADR-0010 gereği çevrilmez.
    BluetoothAgentStatus,
    /// Ayarlar'ı bir sayfada aç (`nuxut-settings <sayfa>`; bildirim düğmeleri). Sayfa adı
    /// [`valid_settings_page`] ile sınırlı; bilinmeyen sayfayı Ayarlar kendisi reddeder.
    OpenSettings(String),
    Ping,
    Version,
    /// Shell düzgün kapanır (çıkış kodu 0; yanıt `ok` kapanmadan önce yazılır). `install.sh`
    /// güncellemeden sonra systemd dışında başlamış örneği yeniden başlatmak için kullanır.
    /// systemd birimi `Restart=on-failure` olduğundan birim altında çalışan shell de kalıcı
    /// olarak durur: birimi yeniden başlatmak için `systemctl --user restart nuxut-shell`.
    Quit,
    Help,
}

/// `hypr …` alt komutları.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HyprCommand {
    /// Son eşitlemenin durumu (tek satır JSON).
    Status,
    /// Yeniden üret + uygula.
    Sync,
    /// `[compositor] ensure_source = false` iken bile `source` satırını bir kez ekle.
    AddSource,
    /// Nuxut satırını ve üretilen dosyayı kaldır (bir daha kendiliğinden eklenmez).
    Undo,
    /// Diskteki ayar dili çalışan oturumunkinden farklıysa (`pending_switch`) şimdi geçir
    /// (`hyprctl reload full-reset`); bildirimdeki "Şimdi uygula" düğmesinin karşılığı.
    ApplySwitch,
    /// En yeni ana ayar yedeğini geri koy (ADR-0007; eşitleme thread'inde, ardından yeniden yükleme).
    RestoreBackup,
    /// `nuxut-shell hypr takeover` bitti: durum dizinindeki rapordan özet bildirimi ("Geri al" =
    /// `hypr restore-backup`).
    TakeoverDone,
}

/// `idle …` alt komutları.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleCommand {
    Status,
    /// Kahve modu: `inhibit` = `On`, `uninhibit` = `Off`, `toggle`.
    Inhibit(Toggle),
}

/// `night-light …` alt komutları.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NightLightCommand {
    Status,
    /// Elle aç/kapa (bir sonraki zamanlama sınırına kadar) ya da gece sıcaklığı.
    Run(crate::services::night_light::Command),
}

fn parse_night_light(rest: &[&str]) -> Result<NightLightCommand, String> {
    use crate::services::night_light::Command as N;
    let range = crate::services::night_light::config::TEMPERATURE;
    Ok(match rest {
        ["status"] => NightLightCommand::Status,
        ["on"] => NightLightCommand::Run(N::Set(true)),
        ["off"] => NightLightCommand::Run(N::Set(false)),
        ["toggle"] => NightLightCommand::Run(N::Toggle),
        ["temperature", k] => match k.parse::<u32>() {
            Ok(v) if range.contains(&v) && !k.starts_with('+') => NightLightCommand::Run(N::Temperature(v)),
            _ => {
                return Err(format!("\"{}\" geçersiz ({}-{} arası tam sayı, Kelvin)", clip(k), range.start(), range.end()));
            }
        },
        _ => return Err("kullanım: night-light <status|on|off|toggle> | night-light temperature <1500-6000>".into()),
    })
}

/// `magnifier …` alt komutları.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MagnifierCommand {
    /// Tek satır JSON (sözleşme magnifier-v1, `services::magnifier::Status`).
    Status,
    Run(crate::services::magnifier::Command),
}

const MAGNIFIER_USAGE: &str = "kullanım: magnifier <status|in|out|reset|on|off|toggle> | magnifier set <1-10>";

fn parse_magnifier(rest: &[&str]) -> Result<MagnifierCommand, String> {
    use crate::services::magnifier::{Command as M, Factor, parse_factor};
    Ok(match rest {
        ["status"] => MagnifierCommand::Status,
        ["in"] => MagnifierCommand::Run(M::In),
        ["out"] => MagnifierCommand::Run(M::Out),
        ["reset"] => MagnifierCommand::Run(M::Reset),
        ["on"] => MagnifierCommand::Run(M::On),
        ["off"] => MagnifierCommand::Run(M::Off),
        ["toggle"] => MagnifierCommand::Run(M::Toggle),
        ["set", f] => match parse_factor(f) {
            Some(v) => MagnifierCommand::Run(M::Set(Factor::new(v))),
            None => return Err(format!("\"{}\" geçersiz (1-10 arası çarpan, ondalık nokta ile: 2.5)", clip(f))),
        },
        _ => return Err(MAGNIFIER_USAGE.into()),
    })
}
/// `power …` alt komutları (sözleşme power-v1; `docs/design/power.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerCommand {
    /// Pil, profiller, şarj sınırı (tek satır JSON).
    Status,
    /// Yalnızca şarj sınırı (tek satır JSON).
    ChargeLimitStatus,
    /// Şarj sınırını aç/kapat; yanıt `ok` ya da `error: <unavailable|unsupported|denied|failed>: …`.
    SetChargeLimit(bool),
}

const POWER_USAGE: &str = "kullanım: power status | power charge-limit <on|off|status>";

/// `osk …` alt komutları.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OskAction {
    /// Tek satır JSON (sözleşme osk-v1, `services::osk::status_json`).
    Status,
    Run(crate::services::osk::Command),
}

const OSK_USAGE: &str = "kullanım: osk <toggle|show|hide> [--output <ad>] | osk status";

fn parse_osk(action: &str) -> Result<crate::services::osk::Command, String> {
    use crate::services::osk::Command as O;
    Ok(match action {
        "toggle" => O::Toggle,
        "show" => O::Show,
        "hide" => O::Hide,
        _ => return Err(OSK_USAGE.into()),
    })
}

/// `setup …` alt komutları.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupCommand {
    /// Tek satır JSON (ADR-0001 §10.2 + `setup_state`).
    Status,
    /// Ayarlar sihirbazı bitti/atlandı: `setup.toml`'a sürümle yaz (düşürmez).
    Record { state: crate::services::setup::SetupState, version: u16 },
}

/// Sihirbaz sürümü: yalnızca ondalık rakamlar, baştaki sıfır/işaret yok, 1..=65535.
fn parse_setup_version(word: &str) -> Result<u16, String> {
    let bad = || format!("\"{}\" geçersiz sürüm (1-65535 arası tam sayı)", clip(word));
    if word.is_empty() || word.len() > 5 || !word.bytes().all(|b| b.is_ascii_digit()) || word.starts_with('0') {
        return Err(bad());
    }
    word.parse::<u16>().map_err(|_| bad())
}

/// `snapshot …` alt komutları (ADR-0001 §9.3). Kimlikler ayrıştırılırken doğrulanır: ayar
/// kimliği kapalı listeden (`services::apply::known_setting`), snapshot kimliği biçimden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotCommand {
    /// Yeniden eskiye; `limit` 1..=200 (varsayılan 50).
    List { setting: Option<nuxut_apply::SettingId>, limit: usize },
    Show(nuxut_apply::SnapshotId),
    /// `force`: sonradan değişmiş parçaların da üzerine yaz; `setting`: snapshot bu ayara ait
    /// değilse hiçbir şey yazılmaz.
    Restore { id: nuxut_apply::SnapshotId, force: bool, setting: Option<nuxut_apply::SettingId> },
}

/// `snapshot list` varsayılan satır sayısı.
pub const SNAPSHOT_LIST_DEFAULT: usize = 50;

fn parse_snapshot_id(word: &str) -> Result<nuxut_apply::SnapshotId, String> {
    nuxut_apply::SnapshotId::parse(word).map_err(|_| format!("invalid: \"{}\" geçersiz snapshot kimliği", clip(word)))
}

fn parse_setting_id(word: &str) -> Result<nuxut_apply::SettingId, String> {
    crate::services::apply::known_setting(word).ok_or_else(|| format!("invalid: \"{}\" bilinmeyen ayar kimliği", clip(word)))
}

const SNAPSHOT_USAGE: &str =
    "kullanım: snapshot list [--setting <ayar>] [--limit <1-200>] | snapshot show <kimlik> | snapshot restore <kimlik> [--force] [--setting <ayar>]";

/// `snapshot …` argümanları (`snapshot` sonrası). Her bayrak en çok bir kez.
fn parse_snapshot(args: &[&str]) -> Result<SnapshotCommand, String> {
    let (sub, rest) = args.split_first().ok_or(SNAPSHOT_USAGE)?;
    let (id, mut rest) = match *sub {
        "list" => (None, rest),
        "show" | "restore" => {
            let (id, rest) = rest.split_first().ok_or(SNAPSHOT_USAGE)?;
            (Some(parse_snapshot_id(id)?), rest)
        }
        _ => return Err(SNAPSHOT_USAGE.into()),
    };
    let (mut setting, mut limit, mut force) = (None, None, false);
    while let Some((flag, tail)) = rest.split_first() {
        rest = tail;
        match (*sub, *flag) {
            ("list" | "restore", "--setting") if setting.is_none() => {
                let (v, tail) = rest.split_first().ok_or(SNAPSHOT_USAGE)?;
                rest = tail;
                setting = Some(parse_setting_id(v)?);
            }
            ("list", "--limit") if limit.is_none() => {
                let (v, tail) = rest.split_first().ok_or(SNAPSHOT_USAGE)?;
                rest = tail;
                let n = v
                    .parse::<usize>()
                    .ok()
                    .filter(|n| (1..=crate::services::apply::MAX_LIST_LIMIT).contains(n) && !v.starts_with(['0', '+']))
                    .ok_or_else(|| format!("invalid: \"{}\" geçersiz sınır (1-200)", clip(v)))?;
                limit = Some(n);
            }
            ("restore", "--force") if !force => force = true,
            _ => return Err(SNAPSHOT_USAGE.into()),
        }
    }
    Ok(match (*sub, id) {
        ("list", _) => SnapshotCommand::List { setting, limit: limit.unwrap_or(SNAPSHOT_LIST_DEFAULT) },
        ("show", Some(id)) => SnapshotCommand::Show(id),
        (_, Some(id)) => SnapshotCommand::Restore { id, force, setting },
        _ => return Err(SNAPSHOT_USAGE.into()),
    })
}

/// `displays …` alt komutları (ADR-0005 §7; JSON şeması `services::displays::protocol`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplaysCommand {
    /// Bağlı monitörler, kipleri ve istenen durum (tek satır JSON).
    List,
    /// `gen`, bekleyen değişiklik, son sonuç (tek satır JSON; Ayarlar 1 sn'de bir yoklar).
    Status,
    /// Tam istenen durum (tek argüman JSON; `MAX_DISPLAYS_ARG` bayta kadar).
    Apply(String),
    Confirm(String),
    Revert(String),
}

/// `displays apply` JSON argümanı üst sınırı (framework IPC satırı 4096 bayt).
pub const MAX_DISPLAYS_ARG: usize = 4000;

/// `displays …` argümanları. JSON tek argümandır (sekme/satır sonu içeremez; boşluk olabilir:
/// framework IPC argümanları sekmeyle ayırır).
fn parse_displays(args: &[String]) -> Result<Command, String> {
    let token = |t: &str| -> Result<String, String> {
        if !t.is_empty() && t.len() <= 32 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(t.to_owned())
        } else {
            Err(format!("geçersiz jeton \"{}\"", clip(t)))
        }
    };
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let cmd = match words.as_slice() {
        ["displays", "list"] => DisplaysCommand::List,
        ["displays", "status"] => DisplaysCommand::Status,
        ["displays", "apply", json] => {
            if json.len() > MAX_DISPLAYS_ARG {
                return Err(format!("too_large: istek en çok {MAX_DISPLAYS_ARG} bayt"));
            }
            if json.chars().any(char::is_control) {
                return Err("bad_json: kontrol karakteri".into());
            }
            DisplaysCommand::Apply((*json).to_owned())
        }
        ["displays", "confirm", t] => DisplaysCommand::Confirm(token(t)?),
        ["displays", "revert", t] => DisplaysCommand::Revert(token(t)?),
        _ => return Err("kullanım: displays <list|status> | displays apply <JSON> | displays <confirm|revert> <jeton>".into()),
    };
    Ok(Command::Displays(cmd))
}

/// `defaults …` alt komutları (sözleşme defaults-v1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultsCommand {
    /// Roller, mevcut seçimler ve adaylar (tek satır JSON v1).
    Status,
    /// Rolün varsayılanını yaz. Rol ve kimlik burada yalnızca biçimce sınırlanır; bilinmeyen rol /
    /// aday olmayan kimlik servisin JSON hata kodudur (`unknown_role`, `unknown_app`).
    Set { role: String, id: String },
    /// Rolün varsayılan uygulamasını başlat (temel katman kısayolları). Kurulu varsayılan yoksa ilk
    /// aday; hiç yoksa `no_app`.
    Open { role: String },
}

const DEFAULTS_USAGE: &str = "kullanım: defaults status | defaults set <rol> <masaüstü-kimliği> | defaults open <rol>";

fn parse_defaults_set(role: &str, id: &str) -> Result<DefaultsCommand, String> {
    if role.len() > 32 || id.len() > 255 {
        return Err(DEFAULTS_USAGE.into());
    }
    Ok(DefaultsCommand::Set { role: role.to_owned(), id: id.to_owned() })
}

/// `look …` alt komutları: `status`/`plan` salt okunur; `restart-app` canlı yenilenmeyen bir
/// uygulamayı yeni renklerle yeniden başlatır.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookCommand {
    /// Seçimler, çözülmüş temalar, Qt yolu, eksik paketler, çakışmalar (tek satır JSON).
    Status,
    /// Birleşik görünüm açıkken yazılacak parçalar ve canlı eylemler (tek satır JSON).
    Plan,
    /// Canlı yenilenmeyen uygulamayı yeni renklerle yeniden başlat (bildirim düğmesi;
    /// `services::look::app_refresh`).
    RestartApp(crate::services::look::app_refresh::App),
    /// Başka bir aracın GTK teması kullanılsın: birleşik görünüm kapatılır (onarma bildirimindeki
    /// "Bu aracı kullan"; Ayarlar › Görünüm'den yeniden açılır).
    UseOtherTool,
}

pub const HELP: &str = "\
komutlar:
  panel <toggle|open|close> <right|left|top|bottom|clock|launcher|sysmon> [--output <ad>]
  launcher <toggle|open|close|clipboard|emoji> [--output <ad>]
  clipboard list | clipboard pick <id> | clipboard delete <id> | clipboard clear
  panel close-all [--output <ad>]
  panel pin <right|left|top|bottom> [toggle|on|off] [--output <ad>]
  bar <toggle|show|hide|focus> [--output <ad>]
  volume <up|down> [<adım%>] | volume set <0-150> | volume mute [toggle|on|off] [--output <ad>]
  mic mute [toggle|on|off] [--output <ad>]
  brightness <up|down> [<adım%>] | brightness set <0-100> [--output <ad>]
  config reload | config status | config undo | config undo-migration | config heal [--dry-run|--undo|--fix-typos|--ignore-typos]
  audio-diagnose
  doctor live                  (iç; rapor için: nuxut-shell doctor)
  hypr <status|sync|add-source|undo|apply-switch|restore-backup|takeover-done>
  displays <list|status> | displays apply <JSON> | displays <confirm|revert> <jeton>
  snapshot list [--setting <ayar>] [--limit <1-200>] | snapshot show <kimlik>
  snapshot restore <kimlik> [--force] [--setting <ayar>]
  setup status | setup <done|skip> <1-65535>
  look <status|plan|use-other-tool> | look restart-app <uygulama>
  idle <status|inhibit|uninhibit|toggle>
  lock
  settings open <sayfa>
  night-light <status|on|off|toggle> | night-light temperature <1500-6000>
  magnifier <status|in|out|reset|on|off|toggle> | magnifier set <1-10>
  osk <toggle|show|hide> [--output <ad>] | osk status
  images status
  wallpaper builtins
  shot region|window|output|screen [--output <ad>] [--no-save] [--no-copy] | shot open|reveal <n>
  input status
  clock refresh
  defaults status | defaults set <rol> <masaüstü-kimliği> | defaults open <rol>
  session menu [--output <ad>]
  removable list | removable open <id> | removable eject <id>
  power status | power charge-limit <on|off|status>
  bluetooth agent-status
  ping | version | quit | help";

/// Satırı komuta çevirir. Hata metni kullanıcıya gösterilir (`error: ` ön eki çağıranın).
pub fn parse(line: &str) -> Result<Command, String> {
    // `displays …`: JSON taşır; genel 512 bayt sınırı yerine framework satır sınırı (4096) ve
    // `MAX_DISPLAYS_ARG` geçerli. Diğer komutlar 512'de kalır.
    if let Some(rest) = line.trim_start().strip_prefix("displays")
        && (rest.is_empty() || rest.starts_with(char::is_whitespace))
    {
        if line.len() > nuxut::wayland::ipc::MAX_LINE {
            return Err(format!("too_large: satır en çok {} bayt", nuxut::wayland::ipc::MAX_LINE));
        }
        if line.chars().any(|c| c.is_control() && c != '\t') {
            return Err("komut kontrol karakteri içeriyor".into());
        }
        let rest = rest.trim_start();
        let mut args = vec!["displays".to_owned()];
        match rest.strip_prefix("apply") {
            Some(json) if json.starts_with(char::is_whitespace) => {
                args.push("apply".into());
                args.push(json.trim().to_owned());
            }
            _ => args.extend(rest.split_whitespace().map(str::to_owned)),
        }
        return parse_displays(&args);
    }
    if line.len() > MAX_LINE {
        return Err(format!("komut çok uzun (en çok {MAX_LINE} bayt)"));
    }
    if line.chars().any(|c| c.is_control() && c != '\t') {
        return Err("komut kontrol karakteri içeriyor".into());
    }
    let mut tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.len() > MAX_TOKENS {
        return Err("çok fazla argüman".into());
    }
    let output = take_output(&mut tokens)?;
    let cmd = match tokens.as_slice() {
        ["panel", "close-all"] => Command::ClosePanels { output },
        ["panel", "pin", target] => Command::PinPanel { edge: parse_edge(target)?, action: Toggle::Toggle, output },
        ["panel", "pin", target, action] => Command::PinPanel { edge: parse_edge(target)?, action: parse_toggle(action, "on", "off")?, output },
        ["panel", action, target] => Command::Panel { panel: parse_panel(target)?, action: parse_toggle(action, "open", "close")?, output },
        ["panel", ..] => return Err("kullanım: panel <toggle|open|close> <right|left|top|bottom|clock|launcher|sysmon>".into()),
        ["launcher", "clipboard"] => Command::LauncherMode { mode: crate::launcher_view::LauncherMode::Clipboard, output },
        ["launcher", "emoji"] => Command::LauncherMode { mode: crate::launcher_view::LauncherMode::Emoji, output },
        ["launcher", action] => {
            Command::Panel { panel: PanelId::Launcher, action: parse_toggle(action, "open", "close")?, output }
        }
        ["launcher", ..] => return Err("kullanım: launcher <toggle|open|close|clipboard|emoji>".into()),
        ["clipboard", rest @ ..] if output.is_none() => Command::Clipboard(crate::services::clipboard::parse_command(rest)?),
        ["clipboard", ..] => return Err(crate::services::clipboard::USAGE.into()),
        ["bar", "focus"] => Command::BarFocus { output },
        ["bar", action] => Command::Bar { action: parse_toggle(action, "show", "hide")?, output },
        ["bar", ..] => return Err("kullanım: bar <toggle|show|hide|focus>".into()),
        ["volume", "mute"] => Command::VolumeMute { action: Toggle::Toggle, output },
        ["volume", "mute", action] => Command::VolumeMute { action: parse_toggle(action, "on", "off")?, output },
        ["volume", rest @ ..] => Command::Volume { level: parse_level(rest, MAX_VOLUME_SET, "volume")?, output },
        ["mic", "mute"] => Command::MicMute { action: Toggle::Toggle, output },
        ["mic", "mute", action] => Command::MicMute { action: parse_toggle(action, "on", "off")?, output },
        ["mic", ..] => return Err("kullanım: mic mute [toggle|on|off]".into()),
        ["brightness", rest @ ..] => Command::Brightness { level: parse_level(rest, 100, "brightness")?, output },
        ["config", "reload"] if output.is_none() => Command::ReloadConfig,
        ["config", "undo-migration"] if output.is_none() => Command::UndoMigration,
        ["config", "status"] if output.is_none() => Command::ConfigStatus,
        ["config", "undo"] if output.is_none() => Command::ConfigUndo,
        ["config", "heal"] if output.is_none() => Command::ConfigHeal(HealCommand::Apply),
        ["config", "heal", "--dry-run"] if output.is_none() => Command::ConfigHeal(HealCommand::DryRun),
        ["config", "heal", "--undo"] if output.is_none() => Command::ConfigHeal(HealCommand::Undo),
        ["config", "heal", "--fix-typos"] if output.is_none() => Command::ConfigHeal(HealCommand::FixTypos),
        ["config", "heal", "--ignore-typos"] if output.is_none() => Command::ConfigHeal(HealCommand::IgnoreTypos),
        ["config", "heal", ..] => return Err("kullanım: config heal [--dry-run|--undo|--fix-typos|--ignore-typos]".into()),
        ["audio-diagnose"] if output.is_none() => Command::AudioDiagnose,
        ["doctor", "live"] if output.is_none() => Command::DoctorLive,
        ["hypr", "status"] if output.is_none() => Command::Hypr(HyprCommand::Status),
        ["hypr", "sync"] if output.is_none() => Command::Hypr(HyprCommand::Sync),
        ["hypr", "add-source"] if output.is_none() => Command::Hypr(HyprCommand::AddSource),
        ["hypr", "undo"] if output.is_none() => Command::Hypr(HyprCommand::Undo),
        ["hypr", "apply-switch"] if output.is_none() => Command::Hypr(HyprCommand::ApplySwitch),
        ["hypr", "restore-backup"] if output.is_none() => Command::Hypr(HyprCommand::RestoreBackup),
        ["hypr", "takeover-done"] if output.is_none() => Command::Hypr(HyprCommand::TakeoverDone),
        ["hypr", ..] => return Err("kullanım: hypr <status|sync|add-source|undo|apply-switch|restore-backup|takeover-done>".into()),
        ["snapshot", rest @ ..] if output.is_none() => Command::Snapshot(parse_snapshot(rest)?),
        ["setup", "status"] if output.is_none() => Command::Setup(SetupCommand::Status),
        ["setup", action @ ("done" | "skip"), version] if output.is_none() => {
            use crate::services::setup::SetupState;
            let state = if *action == "done" { SetupState::Done } else { SetupState::Skipped };
            Command::Setup(SetupCommand::Record { state, version: parse_setup_version(version)? })
        }
        ["setup", ..] => return Err("kullanım: setup status | setup <done|skip> <1-65535>".into()),
        ["look", "status"] if output.is_none() => Command::Look(LookCommand::Status),
        ["look", "plan"] if output.is_none() => Command::Look(LookCommand::Plan),
        ["look", "use-other-tool"] if output.is_none() => Command::Look(LookCommand::UseOtherTool),
        ["look", "restart-app", id] if output.is_none() => match crate::services::look::app_refresh::App::parse(id) {
            Some(app) => Command::Look(LookCommand::RestartApp(app)),
            None => return Err(format!("bilinmeyen uygulama: {}", clip(id))),
        },
        ["look", ..] => return Err("kullanım: look <status|plan|restart-app <uygulama>|use-other-tool>".into()),
        ["idle", "status"] if output.is_none() => Command::Idle(IdleCommand::Status),
        ["idle", "inhibit"] if output.is_none() => Command::Idle(IdleCommand::Inhibit(Toggle::On)),
        ["idle", "uninhibit"] if output.is_none() => Command::Idle(IdleCommand::Inhibit(Toggle::Off)),
        ["idle", "toggle"] if output.is_none() => Command::Idle(IdleCommand::Inhibit(Toggle::Toggle)),
        ["idle", ..] => return Err("kullanım: idle <status|inhibit|uninhibit|toggle>".into()),
        ["night-light", rest @ ..] if output.is_none() => Command::NightLight(parse_night_light(rest)?),
        ["magnifier", rest @ ..] if output.is_none() => Command::Magnifier(parse_magnifier(rest)?),
        ["osk", "status"] if output.is_none() => Command::Osk { action: OskAction::Status, output },
        ["osk", action] => Command::Osk { action: OskAction::Run(parse_osk(action)?), output },
        ["osk", ..] => return Err(OSK_USAGE.into()),
        ["images", "status"] if output.is_none() => Command::ImagesStatus,
        ["images", ..] => return Err("kullanım: images status".into()),
        ["wallpaper", "builtins"] if output.is_none() => Command::WallpaperBuiltins,
        ["wallpaper", ..] => return Err("kullanım: wallpaper builtins".into()),
        ["shot", rest @ ..] => Command::Shot(crate::services::screenshot::parse_command(rest, output)?),
        ["input", "status"] if output.is_none() => Command::InputStatus,
        ["input", ..] => return Err("kullanım: input status".into()),
        ["clock", "refresh"] if output.is_none() => Command::ClockRefresh,
        ["clock", ..] => return Err("kullanım: clock refresh".into()),
        ["defaults", "status"] if output.is_none() => Command::Defaults(DefaultsCommand::Status),
        ["defaults", "set", role, id] if output.is_none() => Command::Defaults(parse_defaults_set(role, id)?),
        ["defaults", "open", role] if output.is_none() && role.len() <= 32 => Command::Defaults(DefaultsCommand::Open { role: (*role).to_owned() }),
        ["session", "menu"] => Command::SessionMenu { output },
        ["session", ..] => return Err("kullanım: session menu".into()),
        ["defaults", ..] => return Err(DEFAULTS_USAGE.into()),
        ["removable", rest @ ..] if output.is_none() => Command::Removable(crate::services::removable_media::parse_command(rest)?),
        ["removable", ..] => return Err(crate::services::removable_media::USAGE.into()),
        ["power", "status"] if output.is_none() => Command::Power(PowerCommand::Status),
        ["power", "charge-limit", "status"] if output.is_none() => Command::Power(PowerCommand::ChargeLimitStatus),
        ["power", "charge-limit", "on"] if output.is_none() => Command::Power(PowerCommand::SetChargeLimit(true)),
        ["power", "charge-limit", "off"] if output.is_none() => Command::Power(PowerCommand::SetChargeLimit(false)),
        ["power", ..] => return Err(POWER_USAGE.into()),
        ["bluetooth", "agent-status"] if output.is_none() => Command::BluetoothAgentStatus,
        ["bluetooth", ..] => return Err("kullanım: bluetooth agent-status".into()),
        ["lock"] if output.is_none() => Command::Lock,
        ["lock", ..] => return Err("kullanım: lock".into()),
        ["settings", "open", page] if output.is_none() && valid_settings_page(page) => Command::OpenSettings(page.to_string()),
        ["settings", ..] => return Err("kullanım: settings open <sayfa>".into()),
        ["ping"] if output.is_none() => Command::Ping,
        ["version"] if output.is_none() => Command::Version,
        ["quit"] if output.is_none() => Command::Quit,
        ["help"] | [] if output.is_none() => Command::Help,
        [first, ..] => return Err(format!("bilinmeyen komut \"{}\" (help ile listele)", clip(first))),
        [] => return Err("--output tek başına kullanılamaz".into()),
    };
    Ok(cmd)
}

/// Ayarlar sayfa adı: küçük harfle başlar, `[a-z0-9-]`, en çok 32 bayt (seçenek gibi görünemez).
pub fn valid_settings_page(page: &str) -> bool {
    page.len() <= 32
        && page.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && page.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn take_output(
tokens: &mut Vec<&str>) -> Result<Option<String>, String> {
    let Some(i) = tokens.iter().position(|t| *t == "--output") else { return Ok(None) };
    let name = tokens.get(i + 1).ok_or("--output bir monitör adı ister")?.to_string();
    if name.len() > MAX_OUTPUT_NAME || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)) {
        return Err(format!("geçersiz monitör adı \"{}\"", clip(&name)));
    }
    tokens.drain(i..=i + 1);
    if tokens.contains(&"--output") {
        return Err("--output birden fazla kez verilmiş".into());
    }
    Ok(Some(name))
}

fn parse_toggle(word: &str, on: &str, off: &str) -> Result<Toggle, String> {
    match word {
        "toggle" => Ok(Toggle::Toggle),
        w if w == on => Ok(Toggle::On),
        w if w == off => Ok(Toggle::Off),
        w => Err(format!("\"{}\" geçersiz (toggle|{on}|{off})", clip(w))),
    }
}

/// `up|down [adım]` (adım 1..=100, varsayılan [`DEFAULT_STEP`]) ya da `set <0..=max>`.
fn parse_level(rest: &[&str], max: u8, name: &str) -> Result<Level, String> {
    let percent = |w: &str, lo: u8, hi: u8| -> Result<u8, String> {
        match w.parse::<u8>() {
            Ok(v) if (lo..=hi).contains(&v) => Ok(v),
            _ => Err(format!("\"{}\" geçersiz ({lo}-{hi} arası tam sayı)", clip(w))),
        }
    };
    Ok(match rest {
        ["up"] => Level::Up(DEFAULT_STEP),
        ["down"] => Level::Down(DEFAULT_STEP),
        ["up", step] => Level::Up(percent(step, 1, 100)?),
        ["down", step] => Level::Down(percent(step, 1, 100)?),
        ["set", value] => Level::Set(percent(value, 0, max)?),
        _ => return Err(format!("kullanım: {name} <up|down> [<adım%>] | {name} set <0-{max}>")),
    })
}

fn parse_panel(word: &str) -> Result<PanelId, String> {
    Ok(match word {
        "clock" => PanelId::Clock,
        "launcher" => PanelId::Launcher,
        "sysmon" => PanelId::Sysmon,
        "right" => PanelId::Edge(Edge::Right),
        "left" => PanelId::Edge(Edge::Left),
        "top" => PanelId::Edge(Edge::Top),
        "bottom" => PanelId::Edge(Edge::Bottom),
        w => return Err(format!("bilinmeyen panel \"{}\" (right|left|top|bottom|clock|launcher|sysmon)", clip(w))),
    })
}

/// Kenar paneli adı (sabitleme yalnızca kenar panellerinde; saat popout'u sabitlenmez).
fn parse_edge(word: &str) -> Result<Edge, String> {
    match parse_panel(word) {
        Ok(PanelId::Edge(e)) => Ok(e),
        _ => Err(format!("\"{}\" sabitlenemez (right|left|top|bottom)", clip(word))),
    }
}

fn clip(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(32).collect()
}

/// Runtime IPC'sinden gelen argüman vektörünü komuta çevirir. Argümanlar boşlukla
/// birleştirilip aynı sözdizimiyle ayrıştırılır (komutlarımızın hiçbir argümanı boşluk içermez).
pub fn parse_args(args: &[String]) -> Result<Command, String> {
    // `displays apply` tek JSON argümanı taşır (boşluk içerebilir); ayrı ayrıştırıcı.
    if args.first().is_some_and(|a| a == "displays") {
        return parse_displays(args);
    }
    if args.iter().any(|a| a.chars().any(char::is_whitespace)) {
        return Err("argümanlar boşluk içeremez".into());
    }
    parse(&args.join(" "))
}

/// Tek örnek IPC'sinde uygulama adı (`$XDG_RUNTIME_DIR/nuxut/nuxut-shell.sock`).
pub const APP_NAME: &str = "nuxut-shell";

/// Sunucunun yanıt penceresi ve `nuxut-shell msg` istemcisinin bekleme süresi (framework
/// varsayılanı 2 sn). Yanıtı ayrı thread'den gelen komutlar bundan uzun sürebilir: `snapshot
/// restore` depo kilidini `nuxut_apply::LOCK_WAIT` (5 sn) bekleyebilir, `snapshot list|show`
/// dconf okumalarına ~2 sn + alt süreç zaman aşımı harcayabilir. Pencere dolunca yanıt düşer ama
/// iş yine yapılır (istemci "başarısız" görür, geri yükleme olmuş olur); bu yüzden en kötü
/// süreden geniş. Anında yanıtlanan komutlar etkilenmez. Shell'e komut gönderen başka istemciler
/// (Ayarlar) `snapshot` için en az bu kadar beklemeli.
pub const REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Test/ikinci örnek adı ortam değişkeni: tanımlıysa IPC adı `nuxut-shell-<ad>` olur; hem
/// çalıştırma hem `msg` buna uyar. Testler kullanıcının çalışan shell'ine asla komut göndermesin
/// diye her test örneği kendi adıyla çalışır.
pub const INSTANCE_ENV: &str = "NUXUT_SHELL_INSTANCE";

/// Etkin IPC uygulama adı. Geçersiz örnek adı (yalnızca `[a-z0-9_-]`, en çok 16) hatadır;
/// sessizce varsayılana düşmek kullanıcının shell'ine yanlışlıkla mesaj göndermek olurdu.
pub fn app_name() -> Result<String, String> {
    app_name_from(std::env::var(INSTANCE_ENV).ok().as_deref())
}

fn app_name_from(instance: Option<&str>) -> Result<String, String> {
    match instance {
        None | Some("") => Ok(APP_NAME.to_owned()),
        Some(s) if s.len() <= 16 && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-') => {
            Ok(format!("{APP_NAME}-{s}"))
        }
        Some(s) => Err(format!("{INSTANCE_ENV}=\"{}\" geçersiz ([a-z0-9_-], en çok 16)", clip(s))),
    }
}

/// `nuxut-shell` argümanları.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cli {
    /// Shell'i çalıştır.
    Run,
    /// Çalışan shell'e komut gönder (argüman vektörü olarak).
    Msg(Vec<String>),
    /// Ayarlar uygulaması için config/widget şemasını (TOML) yazdır; shell gerekmez.
    Schema,
    /// hyprlang ölü adam anahtarı (Hyprland `dispatch exec` ile başlatır; ADR-0005 §5.3):
    /// `display-guard <jeton> <son an unix ms> <örnek>`.
    DisplayGuard(Vec<String>),
    /// Birleşik görünümü tek seferlik, süreç içi uygula (`look apply`; çalışan shell gerekmez,
    /// ona dokunulmaz). Sonuç JSON.
    LookApply,
    /// Görünümü Nuxut öncesine geri al (`look revert`; ADR-0004 §8-3 kapatma yolu, framework.toml'a
    /// bakmaz, çalışan shell'e dokunmaz). install.sh test dönemi taze kurulumunda. Sonuç JSON.
    LookRevert,
    /// Durum raporu (`doctor [--json]`, `msg doctor [--json]` de buraya gelir): istemci süreçte
    /// üretilir, kabuk çalışmasa da çalışır; yalnız okur (docs/design/doctor.md).
    Doctor { json: bool },
    /// Ana Hyprland ayarını devral (`hypr takeover`; ADR-0007): Nuxut'u göstermiyorsa benzersiz
    /// yedek + doğrulanmış yeni dosya. Çalışan shell gerekmez. Sonuç JSON.
    HyprTakeover,
    /// En yeni yedeği geri koy (`hypr restore-backup`; o anki dosya önce yedeklenir). Sonuç JSON.
    HyprRestoreBackup,
    /// Sistem görünümü (Plymouth/GRUB/Limine): `system-theme status|preview|apply|revert …`
    /// (`services::system_theme`; yazma kök yardımcıda, pkexec ile).
    SystemTheme(Vec<String>),
    /// Giriş ekranı (greetd + nuxut-greeter): `login-screen status|apply|revert …`
    /// (`services::login_screen`; yazma kök yardımcının `greeter` alanında, pkexec ile).
    LoginScreen(Vec<String>),
    /// Açılış görünümü + giriş ekranı tek yönetici onayıyla: `system-look status|apply|revert …`
    /// (`services::system_look`; kök yardımcının `system-look` alanı).
    SystemLook(Vec<String>),
    /// Önerilen resmî depo paketleri: `packages install <ad>…` (`services::root_packages`; kurulum
    /// kök yardımcının `packages` alanında, pkexec ile).
    Packages(Vec<String>),
    Help,
    Version,
}

pub const USAGE: &str = "\
kullanım:
  nuxut-shell                 shell'i çalıştırır (tek örnek)
  nuxut-shell msg <komut…>    çalışan shell'e komut gönderir (nuxut-shell msg help)
  nuxut-shell schema          ayarlar şemasını (TOML) yazdırır
  nuxut-shell look apply      birleşik görünümü bir kez uygular / kapalıysa geri alır (JSON)
  nuxut-shell look revert     birleşik görünümü Nuxut öncesine geri alır (ayardan bağımsız; JSON)
  nuxut-shell doctor [--json] kurulum ve oturum durum raporu (yalnız okur; ✗ varsa çıkış 1)
  nuxut-shell hypr takeover    Hyprland ana ayarını devralır (benzersiz yedek, doğrulama; JSON)
  nuxut-shell hypr restore-backup  en yeni yedeği geri koyar (o anki dosya yedeklenir; JSON)
  nuxut-shell system-theme …  önyükleme görünümü (Plymouth/GRUB/Limine): status|preview|apply|revert
  nuxut-shell login-screen …  giriş ekranı (greetd + nuxut-greeter): status|apply|revert
  nuxut-shell system-look …   açılış görünümü + giriş ekranı tek onayla: status|apply|revert
  nuxut-shell packages install <ad>…  önerilen resmî depo paketlerini yönetici onayıyla kur
  nuxut-shell --help | --version";

/// `doctor` argümanları: yalnız `--json`.
fn parse_doctor(rest: &[String]) -> Result<Cli, String> {
    match rest {
        [] => Ok(Cli::Doctor { json: false }),
        [j] if j == "--json" => Ok(Cli::Doctor { json: true }),
        _ => Err(format!("kullanım: doctor [--json]\n{USAGE}")),
    }
}

pub fn parse_cli<I: IntoIterator<Item = String>>(args: I) -> Result<Cli, String> {
    let args: Vec<String> = args.into_iter().collect();
    match args.first().map(String::as_str) {
        None => Ok(Cli::Run),
        // `msg doctor`: rapor istemcide üretilir (terminalin PATH'i, kurulu dosyalar); kabuğa yalnız
        // `doctor live` gider.
        Some("msg") if args.get(1).map(String::as_str) == Some("doctor") => parse_doctor(&args[2..]),
        Some("doctor") => parse_doctor(&args[1..]),
        Some("msg") => {
            let rest = args[1..].to_vec();
            // Gönderen tarafta da doğrula: yanlış komut sunucuya gitmeden açıklansın.
            parse_args(&rest)?;
            Ok(Cli::Msg(rest))
        }
        Some("schema") if args.len() == 1 => Ok(Cli::Schema),
        Some("display-guard") => Ok(Cli::DisplayGuard(args[1..].to_vec())),
        Some("schema") => Err(format!("schema argüman almaz\n{USAGE}")),
        Some("look") if args.len() == 2 && args[1] == "apply" => Ok(Cli::LookApply),
        Some("look") if args.len() == 2 && args[1] == "revert" => Ok(Cli::LookRevert),
        Some("hypr") if args.len() == 2 && args[1] == "takeover" => Ok(Cli::HyprTakeover),
        Some("hypr") if args.len() == 2 && args[1] == "restore-backup" => Ok(Cli::HyprRestoreBackup),
        Some("hypr") => Err(format!("kullanım: hypr <takeover|restore-backup> (çalışan kabuk için: msg hypr …)\n{USAGE}")),
        Some("look") => Err(format!("kullanım: look <apply|revert> (durum için: msg look status)\n{USAGE}")),
        Some("system-theme") => Ok(Cli::SystemTheme(args[1..].to_vec())),
        Some("login-screen") => Ok(Cli::LoginScreen(args[1..].to_vec())),
        Some("system-look") => Ok(Cli::SystemLook(args[1..].to_vec())),
        Some("packages") => Ok(Cli::Packages(args[1..].to_vec())),
        Some("-h" | "--help" | "help") => Ok(Cli::Help),
        Some("-V" | "--version") => Ok(Cli::Version),
        Some(other) => Err(format!("bilinmeyen argüman \"{}\"\n{USAGE}", clip(other))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(line: &str) -> Command {
        parse(line).unwrap_or_else(|e| panic!("{line}: {e}"))
    }

    #[test]
    fn panel_commands() {
        assert_eq!(
            ok("panel toggle right"),
            Command::Panel { panel: PanelId::Edge(Edge::Right), action: Toggle::Toggle, output: None }
        );
        assert_eq!(
            ok("  panel   open clock  --output DP-1 "),
            Command::Panel { panel: PanelId::Clock, action: Toggle::On, output: Some("DP-1".into()) }
        );
        assert_eq!(
            ok("panel --output HDMI-A-2 close left"),
            Command::Panel { panel: PanelId::Edge(Edge::Left), action: Toggle::Off, output: Some("HDMI-A-2".into()) }
        );
        assert_eq!(ok("panel close-all"), Command::ClosePanels { output: None });
        assert_eq!(ok("launcher toggle"), Command::Panel { panel: PanelId::Launcher, action: Toggle::Toggle, output: None });
        assert_eq!(
            ok("launcher open --output DP-1"),
            Command::Panel { panel: PanelId::Launcher, action: Toggle::On, output: Some("DP-1".into()) }
        );
        assert_eq!(ok("panel close launcher"), Command::Panel { panel: PanelId::Launcher, action: Toggle::Off, output: None });
        assert!(parse("launcher").is_err());
        assert!(parse("launcher run firefox").is_err());
        use crate::launcher_view::LauncherMode as M;
        assert_eq!(ok("launcher clipboard"), Command::LauncherMode { mode: M::Clipboard, output: None });
        assert_eq!(ok("launcher emoji"), Command::LauncherMode { mode: M::Emoji, output: None });
        assert_eq!(ok("launcher emoji --output DP-1"), Command::LauncherMode { mode: M::Emoji, output: Some("DP-1".into()) });
        assert!(parse("launcher emoji x").is_err());
        assert_eq!(ok("launcher clipboard --output DP-1"), Command::LauncherMode { mode: M::Clipboard, output: Some("DP-1".into()) });
    }

    #[test]
    fn clipboard_commands() {
        use crate::services::clipboard::ClipboardCommand as C;
        assert_eq!(ok("clipboard list"), Command::Clipboard(C::List));
        assert_eq!(ok("clipboard pick 3"), Command::Clipboard(C::Pick(3)));
        assert_eq!(ok("clipboard delete 42"), Command::Clipboard(C::Delete(42)));
        assert_eq!(ok("clipboard clear"), Command::Clipboard(C::Clear));
        for bad in ["clipboard", "clipboard pick", "clipboard pick x", "clipboard list --output DP-1", "clipboard wipe"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn pin_commands() {
        assert_eq!(ok("panel pin right"), Command::PinPanel { edge: Edge::Right, action: Toggle::Toggle, output: None });
        assert_eq!(
            ok("panel pin left on --output DP-1"),
            Command::PinPanel { edge: Edge::Left, action: Toggle::On, output: Some("DP-1".into()) }
        );
        assert_eq!(ok("panel pin top off"), Command::PinPanel { edge: Edge::Top, action: Toggle::Off, output: None });
        assert!(parse("panel pin clock").unwrap_err().contains("sabitlenemez"));
        assert!(parse("panel pin right maybe").is_err());
        assert!(parse("panel pin").is_err());
    }

    #[test]
    fn bar_and_misc_commands() {
        assert_eq!(ok("bar toggle"), Command::Bar { action: Toggle::Toggle, output: None });
        assert_eq!(ok("bar hide --output eDP-1"), Command::Bar { action: Toggle::Off, output: Some("eDP-1".into()) });
        assert_eq!(ok("config reload"), Command::ReloadConfig);
        assert_eq!(ok("config undo-migration"), Command::UndoMigration);
        assert_eq!(ok("config status"), Command::ConfigStatus);
        assert_eq!(ok("config undo"), Command::ConfigUndo);
        assert!(parse("config undo --output DP-1").is_err());
        assert_eq!(ok("config heal"), Command::ConfigHeal(HealCommand::Apply));
        assert_eq!(ok("config heal --dry-run"), Command::ConfigHeal(HealCommand::DryRun));
        assert_eq!(ok("config heal --undo"), Command::ConfigHeal(HealCommand::Undo));
        assert_eq!(ok("config heal --fix-typos"), Command::ConfigHeal(HealCommand::FixTypos));
        assert_eq!(ok("config heal --ignore-typos"), Command::ConfigHeal(HealCommand::IgnoreTypos));
        assert!(parse("config heal --bogus").is_err());
        assert_eq!(ok("audio-diagnose"), Command::AudioDiagnose);
        assert!(parse("audio-diagnose --output DP-1").is_err());
        assert_eq!(ok("doctor live"), Command::DoctorLive);
        assert!(parse("doctor").is_err());
        assert_eq!(ok("hypr status"), Command::Hypr(HyprCommand::Status));
        assert_eq!(ok("hypr sync"), Command::Hypr(HyprCommand::Sync));
        assert_eq!(ok("hypr add-source"), Command::Hypr(HyprCommand::AddSource));
        assert_eq!(ok("hypr apply-switch"), Command::Hypr(HyprCommand::ApplySwitch));
        assert!(parse("hypr").is_err() && parse("hypr reload").is_err() && parse("hypr status --output DP-1").is_err());
        assert_eq!(ok("look status"), Command::Look(LookCommand::Status));
        assert_eq!(ok("look plan"), Command::Look(LookCommand::Plan));
        assert_eq!(ok("look restart-app nautilus"), Command::Look(LookCommand::RestartApp(crate::services::look::app_refresh::App::Nautilus)));
        assert!(parse("look restart-app firefox").is_err() && parse("look restart-app").is_err());
        assert_eq!(ok("look use-other-tool"), Command::Look(LookCommand::UseOtherTool));
        assert_eq!(ok("idle status"), Command::Idle(IdleCommand::Status));
        assert_eq!(ok("idle inhibit"), Command::Idle(IdleCommand::Inhibit(Toggle::On)));
        assert_eq!(ok("idle uninhibit"), Command::Idle(IdleCommand::Inhibit(Toggle::Off)));
        assert_eq!(ok("idle toggle"), Command::Idle(IdleCommand::Inhibit(Toggle::Toggle)));
        assert!(parse("idle").is_err() && parse("idle on").is_err() && parse("idle status --output DP-1").is_err());
        assert_eq!(ok("power status"), Command::Power(PowerCommand::Status));
        assert_eq!(ok("power charge-limit status"), Command::Power(PowerCommand::ChargeLimitStatus));
        assert_eq!(ok("power charge-limit on"), Command::Power(PowerCommand::SetChargeLimit(true)));
        assert_eq!(ok("power charge-limit off"), Command::Power(PowerCommand::SetChargeLimit(false)));
        for bad in ["power", "power charge-limit", "power charge-limit toggle", "power charge-limit 80", "power status --output DP-1"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        {
            use crate::services::night_light::Command as N;
            let nl = |c| Command::NightLight(NightLightCommand::Run(c));
            assert_eq!(ok("night-light status"), Command::NightLight(NightLightCommand::Status));
            assert_eq!(ok("night-light on"), nl(N::Set(true)));
            assert_eq!(ok("night-light off"), nl(N::Set(false)));
            assert_eq!(ok("night-light toggle"), nl(N::Toggle));
            assert_eq!(ok("night-light temperature 3500"), nl(N::Temperature(3500)));
            for bad in [
                "night-light",
                "night-light enable",
                "night-light temperature",
                "night-light temperature 1499",
                "night-light temperature 6001",
                "night-light temperature +4000",
                "night-light temperature 4000K",
                "night-light temperature 4000 5000",
                "night-light on --output DP-1",
            ] {
                assert!(parse(bad).is_err(), "kabul edilmemeli: {bad}");
            }
        }
        {
            use crate::services::magnifier::{Command as M, Factor};
            let mg = |c| Command::Magnifier(MagnifierCommand::Run(c));
            assert_eq!(ok("magnifier status"), Command::Magnifier(MagnifierCommand::Status));
            assert_eq!(ok("magnifier in"), mg(M::In));
            assert_eq!(ok("magnifier out"), mg(M::Out));
            assert_eq!(ok("magnifier reset"), mg(M::Reset));
            assert_eq!(ok("magnifier toggle"), mg(M::Toggle));
            assert_eq!(ok("magnifier on"), mg(M::On));
            assert_eq!(ok("magnifier off"), mg(M::Off));
            assert_eq!(ok("magnifier set 2.5"), mg(M::Set(Factor::new(2.5))));
            assert_eq!(ok("magnifier set 1"), mg(M::Set(Factor::new(1.0))));
            assert_eq!(ok("magnifier set 10"), mg(M::Set(Factor::new(10.0))));
            for bad in [
                "magnifier",
                "magnifier zoom",
                "magnifier set",
                "magnifier set 0.9",
                "magnifier set 10.5",
                "magnifier set -2",
                "magnifier set +2",
                "magnifier set 2,5",
                "magnifier set NaN",
                "magnifier set inf",
                "magnifier set 1e1",
                "magnifier set 2 3",
                "magnifier in 2",
                "magnifier in --output DP-1",
            ] {
                assert!(parse(bad).is_err(), "kabul edilmemeli: {bad}");
            }
        }
        {
            use crate::services::osk::Command as O;
            let osk = |c, o: Option<&str>| Command::Osk { action: OskAction::Run(c), output: o.map(str::to_owned) };
            assert_eq!(ok("osk status"), Command::Osk { action: OskAction::Status, output: None });
            assert_eq!(ok("osk toggle"), osk(O::Toggle, None));
            assert_eq!(ok("osk show --output DP-1"), osk(O::Show, Some("DP-1")));
            assert_eq!(ok("osk hide"), osk(O::Hide, None));
            for bad in ["osk", "osk open", "osk toggle now", "osk status --output DP-1", "osk SHOW"] {
                assert!(parse(bad).is_err(), "kabul edilmemeli: {bad}");
            }
        }
        assert_eq!(ok("images status"), Command::ImagesStatus);
        assert!(parse("images").is_err() && parse("images clear").is_err() && parse("images status --output DP-1").is_err());
        assert_eq!(ok("wallpaper builtins"), Command::WallpaperBuiltins);
        assert!(parse("wallpaper").is_err() && parse("wallpaper set x").is_err() && parse("wallpaper builtins --output DP-1").is_err());
        use crate::services::screenshot::{ShotCommand, ShotMode};
        assert_eq!(
            ok("shot output --output DP-1 --no-copy"),
            Command::Shot(ShotCommand::Capture { mode: ShotMode::Output, output: Some("DP-1".into()), no_save: false, no_copy: true })
        );
        assert_eq!(ok("shot open 2"), Command::Shot(ShotCommand::Open(2)));
        assert!(parse("shot").is_err() && parse("shot window --output DP-1").is_err());
        assert_eq!(ok("input status"), Command::InputStatus);
        assert_eq!(ok("clock refresh"), Command::ClockRefresh);
        assert_eq!(ok("lock"), Command::Lock);
        assert_eq!(ok("settings open power"), Command::OpenSettings("power".into()));
        for bad in ["settings", "settings open", "settings open --flow", "settings open Power", "settings open power x", "settings open power --output DP-1"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert!(parse("lock now").is_err() && parse("lock --output DP-1").is_err());
        assert!(parse("clock").is_err() && parse("clock set").is_err() && parse("clock refresh --output DP-1").is_err());
        use crate::services::removable_media::MediaCommand;
        assert_eq!(ok("removable list"), Command::Removable(MediaCommand::List));
        assert_eq!(ok("removable open sdb1"), Command::Removable(MediaCommand::Open("sdb1".into())));
        assert_eq!(ok("removable eject mmcblk0p1"), Command::Removable(MediaCommand::Eject("mmcblk0p1".into())));
        for bad in ["removable", "removable open", "removable open ../sda", "removable eject sdb1 x", "removable open sdb1 --output DP-1"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert!(parse("input").is_err() && parse("input set").is_err() && parse("input status --output DP-1").is_err());
        assert_eq!(parse_args(&["input".to_owned(), "status".to_owned()]), Ok(Command::InputStatus));
        assert_eq!(ok("defaults status"), Command::Defaults(DefaultsCommand::Status));
        assert_eq!(ok("defaults open terminal"), Command::Defaults(DefaultsCommand::Open { role: "terminal".into() }));
        assert!(parse("defaults open").is_err());
        assert_eq!(ok("session menu"), Command::SessionMenu { output: None });
        assert_eq!(ok("session menu --output DP-1"), Command::SessionMenu { output: Some("DP-1".into()) });
        assert!(parse("session lock").is_err());
        assert_eq!(
            ok("defaults set browser firefox.desktop"),
            Command::Defaults(DefaultsCommand::Set { role: "browser".into(), id: "firefox.desktop".into() })
        );
        assert!(parse("defaults").is_err() && parse("defaults set browser").is_err() && parse("defaults status --output DP-1").is_err());
        assert!(parse(&format!("defaults set browser {}", "a".repeat(300))).is_err());
        assert!(parse("look").is_err() && parse("look apply").is_err() && parse("look status --output DP-1").is_err());
        assert_eq!(ok("bluetooth agent-status"), Command::BluetoothAgentStatus);
        assert!(parse("bluetooth").is_err() && parse("bluetooth agent-status --output DP-1").is_err());
        assert_eq!(ok("ping"), Command::Ping);
        assert_eq!(ok("version"), Command::Version);
        assert_eq!(ok("quit"), Command::Quit);
        assert!(parse("quit --output DP-1").is_err() && parse("quit now").is_err());
        assert_eq!(ok(""), Command::Help);
    }

    #[test]
    fn snapshot_commands() {
        let id = nuxut_apply::SnapshotId::parse("20261010T120000Z-3fa1").unwrap();
        let s = |x: &str| crate::services::apply::known_setting(x);
        assert_eq!(ok("snapshot list"), Command::Snapshot(SnapshotCommand::List { setting: None, limit: 50 }));
        assert_eq!(
            ok("snapshot list --limit 5 --setting hypr.source"),
            Command::Snapshot(SnapshotCommand::List { setting: s("hypr.source"), limit: 5 })
        );
        assert_eq!(ok("snapshot show 20261010T120000Z-3fa1"), Command::Snapshot(SnapshotCommand::Show(id.clone())));
        assert_eq!(
            ok("snapshot restore 20261010T120000Z-3fa1"),
            Command::Snapshot(SnapshotCommand::Restore { id: id.clone(), force: false, setting: None })
        );
        assert_eq!(
            ok("snapshot restore 20261010T120000Z-3fa1 --setting migration.shell --force"),
            Command::Snapshot(SnapshotCommand::Restore { id, force: true, setting: s("migration.shell") })
        );
        for bad in [
            "snapshot",
            "snapshot nope",
            "snapshot show",
            "snapshot show x",
            "snapshot show ../../etc",
            "snapshot show 20261010T120000Z-3fa1 extra",
            "snapshot restore 20261010T120000Z-3fa1 --force --force",
            "snapshot restore 20261010T120000Z-3fa1 --setting",
            "snapshot restore 20261010T120000Z-3fa1 --setting displays",
            "snapshot restore 20261010T120000Z-3fa1 --limit 3",
            "snapshot list --setting hypr.look",
            "snapshot list --setting a --setting b",
            "snapshot list --limit 0",
            "snapshot list --limit 201",
            "snapshot list --limit 05",
            "snapshot list --limit -1",
            "snapshot list --limit 99999999999999999999999",
            "snapshot list --force",
            "snapshot list --output DP-1",
        ] {
            assert!(parse(bad).is_err(), "kabul edilmemeli: {bad}");
        }
        // Boyut sınırı: uzun kimlik satırı reddedilir, panik yok.
        assert!(parse(&format!("snapshot show {}", "a".repeat(600))).is_err());
        assert!(parse_args(&["snapshot".into(), "show".into(), "a b".into()]).is_err());
    }

    #[test]
    fn setup_commands() {
        use crate::services::setup::SetupState;
        assert_eq!(ok("setup status"), Command::Setup(SetupCommand::Status));
        assert_eq!(ok("setup done 1"), Command::Setup(SetupCommand::Record { state: SetupState::Done, version: 1 }));
        assert_eq!(ok("setup skip 65535"), Command::Setup(SetupCommand::Record { state: SetupState::Skipped, version: 65535 }));
        for bad in [
            "setup",
            "setup done",
            "setup skip",
            "setup done 0",
            "setup done 65536",
            "setup done 99999",
            "setup done 100000",
            "setup done -1",
            "setup done +1",
            "setup done 01",
            "setup done 1.0",
            "setup done 1e3",
            "setup done ١",
            "setup done 1 2",
            "setup finish 1",
            "setup status now",
            "setup status --output DP-1",
            "setup done 1 --output DP-1",
        ] {
            assert!(parse(bad).is_err(), "kabul edilmemeli: {bad:?}");
        }
        let cli = parse_cli(["msg", "setup", "done", "3"].iter().map(|s| s.to_string()));
        assert!(matches!(cli, Ok(Cli::Msg(_))));
        assert!(parse_cli(["msg", "setup", "done", "x"].iter().map(|s| s.to_string())).is_err());
    }

    #[test]
    fn volume_and_brightness_commands() {
        assert_eq!(ok("volume up"), Command::Volume { level: Level::Up(DEFAULT_STEP), output: None });
        assert_eq!(ok("volume down 10"), Command::Volume { level: Level::Down(10), output: None });
        assert_eq!(ok("volume set 150"), Command::Volume { level: Level::Set(150), output: None });
        assert_eq!(ok("volume set 0"), Command::Volume { level: Level::Set(0), output: None });
        assert_eq!(ok("volume mute toggle"), Command::VolumeMute { action: Toggle::Toggle, output: None });
        assert_eq!(ok("volume mute"), Command::VolumeMute { action: Toggle::Toggle, output: None });
        assert_eq!(ok("mic mute"), Command::MicMute { action: Toggle::Toggle, output: None });
        assert_eq!(
            ok("mic mute on --output DP-1"),
            Command::MicMute { action: Toggle::On, output: Some("DP-1".into()) }
        );
        assert_eq!(ok("brightness up 1"), Command::Brightness { level: Level::Up(1), output: None });
        assert_eq!(
            ok("brightness set 100 --output eDP-1"),
            Command::Brightness { level: Level::Set(100), output: Some("eDP-1".into()) }
        );
        for bad in [
            "volume",
            "volume up 0",
            "volume up 101",
            "volume down -5",
            "volume up 5.5",
            "volume set 151",
            "volume set",
            "volume up 5 6",
            "volume mute maybe",
            "mic up",
            "brightness set 101",
            "brightness down 300",
            "brightness sideways",
        ] {
            assert!(parse(bad).is_err(), "kabul edilmemeli: {bad:?}");
        }
    }

    #[test]
    fn rejects_bad_input() {
        for bad in [
            "panel toggle middle",
            "panel flip right",
            "panel toggle",
            "bar",
            "bar open",
            "rm -rf /",
            "panel toggle right --output",
            "panel toggle right --output a/b",
            "panel toggle right --output a --output b",
            "ping --output DP-1",
            "--output DP-1",
            "a b c d e f g h i",
            "panel toggle right\u{1b}[2J",
        ] {
            assert!(parse(bad).is_err(), "kabul edilmemeli: {bad:?}");
        }
        assert!(parse(&"x".repeat(MAX_LINE + 1)).is_err());
    }

    #[test]
    fn error_messages_are_clipped() {
        let e = parse(&format!("{} y", "z".repeat(200))).unwrap_err();
        assert!(e.len() < 100, "{e}");
    }

    #[test]
    fn instance_name_isolates_ipc() {
        assert_eq!(app_name_from(None).unwrap(), "nuxut-shell");
        assert_eq!(app_name_from(Some("")).unwrap(), "nuxut-shell");
        assert_eq!(app_name_from(Some("deneme-1")).unwrap(), "nuxut-shell-deneme-1");
        for bad in ["Shell", "a/b", "x y", "çok", "abcdefghijklmnopq"] {
            assert!(app_name_from(Some(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn cli_modes() {
        let cli = |a: &[&str]| parse_cli(a.iter().map(|s| s.to_string()));
        assert_eq!(cli(&[]), Ok(Cli::Run));
        assert_eq!(
            cli(&["msg", "panel", "toggle", "right"]),
            Ok(Cli::Msg(vec!["panel".into(), "toggle".into(), "right".into()]))
        );
        assert!(cli(&["msg", "panel toggle", "right"]).is_err(), "boşluklu argüman");
        assert_eq!(cli(&["--help"]), Ok(Cli::Help));
        assert_eq!(cli(&["--version"]), Ok(Cli::Version));
        assert_eq!(cli(&["schema"]), Ok(Cli::Schema));
        assert_eq!(cli(&["doctor"]), Ok(Cli::Doctor { json: false }));
        assert_eq!(cli(&["doctor", "--json"]), Ok(Cli::Doctor { json: true }));
        assert_eq!(cli(&["msg", "doctor"]), Ok(Cli::Doctor { json: false }), "msg doctor istemcide üretilir");
        assert_eq!(cli(&["msg", "doctor", "--json"]), Ok(Cli::Doctor { json: true }));
        assert!(cli(&["doctor", "x"]).is_err());
        assert!(cli(&["schema", "x"]).is_err());
        assert!(cli(&["msg", "panel", "toggle", "nowhere"]).is_err());
        assert!(cli(&["--frobnicate"]).is_err());
        assert_eq!(cli(&["display-guard", "ab", "1", "x"]), Ok(Cli::DisplayGuard(vec!["ab".into(), "1".into(), "x".into()])));
        assert_eq!(cli(&["look", "apply"]), Ok(Cli::LookApply));
        assert_eq!(cli(&["look", "revert"]), Ok(Cli::LookRevert));
        for bad in [&["look"][..], &["look", "status"], &["look", "apply", "now"], &["look", "revert", "x"]] {
            assert!(cli(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn displays_commands() {
        let args = |a: &[&str]| parse_args(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(args(&["displays", "list"]), Ok(Command::Displays(DisplaysCommand::List)));
        assert_eq!(args(&["displays", "status"]), Ok(Command::Displays(DisplaysCommand::Status)));
        let json = r#"{"v":1,"id":"Dell Inc.|DELL U2720Q|ABC123"}"#;
        assert_eq!(args(&["displays", "apply", json]), Ok(Command::Displays(DisplaysCommand::Apply(json.into()))));
        assert_eq!(args(&["displays", "confirm", "a1B2"]), Ok(Command::Displays(DisplaysCommand::Confirm("a1B2".into()))));
        assert_eq!(args(&["displays", "revert", "ff"]), Ok(Command::Displays(DisplaysCommand::Revert("ff".into()))));
        assert!(args(&["displays", "confirm", "zz"]).is_err());
        assert!(args(&["displays", "confirm"]).is_err());
        assert!(args(&["displays", "apply", "{\n}"]).is_err());
        assert!(args(&["displays", "apply", &"x".repeat(MAX_DISPLAYS_ARG + 1)]).unwrap_err().starts_with("too_large"));
        assert!(args(&["displays", "reboot"]).is_err());
        // Satır biçimi (`parse`): yalnızca `displays` 512 bayt sınırını aşabilir.
        let pad = |n: usize| format!(r#"{{"v":1,"base_gen":1,"pad":"{}"}}"#, "x".repeat(n));
        let ok_json = pad(3900);
        assert_eq!(ok(&format!("displays apply {ok_json}")), Command::Displays(DisplaysCommand::Apply(ok_json.clone())));
        assert_eq!(args(&["displays", "apply", &ok_json]), Ok(Command::Displays(DisplaysCommand::Apply(ok_json.clone()))));
        assert!(parse(&format!("displays apply {}", pad(4100))).unwrap_err().starts_with("too_large"));
        assert!(args(&["displays", "apply", &pad(4100)]).unwrap_err().starts_with("too_large"));
        assert!(parse(&format!("displays apply {}\u{1b}", pad(10))).is_err());
        assert!(parse(&format!("panel toggle right {}", "x".repeat(600))).unwrap_err().contains("çok uzun"));
        assert!(parse(&format!("displaysx {}", "x".repeat(600))).unwrap_err().contains("çok uzun"));
        assert_eq!(ok("displays status"), Command::Displays(DisplaysCommand::Status));
        assert_eq!(ok("  displays   confirm  ab12 "), Command::Displays(DisplaysCommand::Confirm("ab12".into())));
        // CLI tarafı da aynı ayrıştırıcıdan geçer.
        let cli = parse_cli(["msg", "displays", "apply", json].iter().map(|s| s.to_string()));
        assert!(matches!(cli, Ok(Cli::Msg(_))));
    }
}
