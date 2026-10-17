//! Ses servisi: özel PipeWire daemon'u (kendi çalışma dizini ve yapılandırması; kullanıcının
//! ses sunucusuna dokunulmaz) üzerinde null-sink düğümleriyle uçtan uca.
//!
//! Kapsam: ilk okuma, varsayılan cihaz (metadata), dışarıdan seviye değişimi ve OSD, seviye /
//! göreli seviye / sessiz yazma, varsayılan cihaz seçme, uygulama akışı, PipeWire'ın yeniden
//! başlaması. Yardımcı araçlar (`pw-dump`, `pw-cli`, `pw-metadata`) yalnızca testte ve yalnızca
//! özel daemon'a (`PIPEWIRE_REMOTE`) karşı çalışır.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use nuxut_shell::services::audio::{
    AudioBackend, AudioEvent, AudioOptions, AudioService, AudioState, AudioTarget, OsdKind,
};
use crate::support::{Events, PrivateBus, WAIT, channel, skip, wait_until};

const CONFIG: &str = r#"
context.properties = {
    core.daemon = true
    core.name = pipewire-0
    support.dbus = false
    log.level = 1
    mem.allow-mlock = false
}
context.spa-libs = {
    audio.convert.* = audioconvert/libspa-audioconvert
    support.*       = support/libspa-support
}
context.modules = [
    { name = libpipewire-module-protocol-native }
    { name = libpipewire-module-access }
    { name = libpipewire-module-client-node }
    { name = libpipewire-module-adapter }
    { name = libpipewire-module-metadata }
]
context.objects = [
    { factory = metadata args = { metadata.name = default } }
    { factory = adapter args = {
        factory.name = support.null-audio-sink
        node.name = "test.sink.a"
        node.description = "Hoparlör"
        media.class = "Audio/Sink"
        audio.position = [ FL FR ]
    } }
    { factory = adapter args = {
        factory.name = support.null-audio-sink
        node.name = "test.source.a"
        node.description = "Mikrofon"
        media.class = "Audio/Source/Virtual"
        audio.position = [ MONO ]
    } }
]
"#;

/// Özel PipeWire daemon'u. Düşürülünce öldürülür.
struct PrivatePipeWire {
    dir: tempfile::TempDir,
    child: Option<Child>,
}

impl PrivatePipeWire {
    /// Çalıştırılamazsa panikler; yalnızca `NUXUT_SKIP_PIPEWIRE_TESTS=1` ise `None` (atla).
    fn start() -> Option<Self> {
        if let Err(e) = Command::new("pipewire").arg("--version").stdout(Stdio::null()).status() {
            return skip::skip_or_panic("pipewire", skip::SKIP_PIPEWIRE, e);
        }
        // Soket yolu 108 baytı geçmemeli: kısa geçici dizin.
        let dir = tempfile::Builder::new().prefix("nxpw").tempdir_in("/tmp").unwrap();
        std::fs::write(dir.path().join("test.conf"), CONFIG).unwrap();
        let mut pw = Self { dir, child: None };
        pw.spawn();
        Some(pw)
    }

    fn spawn(&mut self) {
        let d = self.dir.path();
        let child = Command::new("pipewire")
            .arg("-c")
            .arg(d.join("test.conf"))
            .env("XDG_RUNTIME_DIR", d)
            .env("PIPEWIRE_RUNTIME_DIR", d)
            .env("PIPEWIRE_CONFIG_DIR", d)
            .env("XDG_CONFIG_HOME", d)
            .env_remove("PIPEWIRE_REMOTE")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        self.child = Some(child);
        let socket = self.socket();
        wait_until("pipewire soketi", || socket.exists());
    }

    fn kill(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_file(self.socket());
        let _ = std::fs::remove_file(self.dir.path().join("pipewire-0.lock"));
        let _ = std::fs::remove_file(self.dir.path().join("pipewire-0-manager"));
        let _ = std::fs::remove_file(self.dir.path().join("pipewire-0-manager.lock"));
    }

    fn socket(&self) -> PathBuf {
        self.dir.path().join("pipewire-0")
    }

    /// Özel daemon'a karşı bir PipeWire aracı çalıştırır.
    fn tool(&self, program: &str, args: &[&str]) -> String {
        let out = Command::new("timeout")
            .arg("5")
            .arg(program)
            .args(args)
            .env("PIPEWIRE_REMOTE", self.socket())
            .env("XDG_RUNTIME_DIR", self.dir.path())
            .stderr(Stdio::null())
            .output()
            .unwrap();
        assert!(out.status.success(), "{program} {args:?} başarısız");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// Düğümün `Props` parametresi (`pw-dump`).
    fn props(&self, id: u32) -> serde_json::Value {
        let dump: serde_json::Value = serde_json::from_str(&self.tool("pw-dump", &[&id.to_string()])).unwrap();
        dump[0]["info"]["params"]["Props"][0].clone()
    }

    fn volumes(&self, id: u32) -> Vec<f64> {
        self.props(id)["channelVolumes"].as_array().map(|a| a.iter().filter_map(|v| v.as_f64()).collect()).unwrap_or_default()
    }

    fn metadata(&self) -> String {
        self.tool("pw-metadata", &["-n", "default"])
    }
}

impl Drop for PrivatePipeWire {
    fn drop(&mut self) {
        self.kill();
    }
}

fn start(socket: &Path, backend: AudioBackend) -> (AudioService, Events<AudioEvent>) {
    let (sink, events) = channel();
    let options = AudioOptions {
        remote: Some(socket.to_string_lossy().into_owned()),
        max_volume: 1.0,
        backend,
        coalesce: Duration::from_millis(10),
        max_backoff: Duration::from_millis(200),
    };
    (AudioService::spawn(options, sink).unwrap(), events)
}

fn state(events: &Events<AudioEvent>, what: &str, pred: impl Fn(&AudioState) -> bool) -> AudioState {
    match events.wait_for(what, |e| matches!(e, AudioEvent::State(s) if pred(s))) {
        AudioEvent::State(s) => s,
        _ => unreachable!(),
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

/// Koşul gerçekleşene dek sorgular (araç çağrıları için; `wait_until` 10 ms aralıkla döner).
fn eventually(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    panic!("koşul gerçekleşmedi: {what}");
}

/// Düğümün tüm kanalları `target` doğrusal seviyesine ulaşana dek bekler; olmazsa son görülen
/// değerle düşer (kararsızlığın nedenini ayırt etmek için: eski değer mi, hiç yazılmadı mı).
fn volumes_reach(pw: &PrivatePipeWire, id: u32, what: &str, target: f64) {
    let mut last = Vec::new();
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        last = pw.volumes(id);
        if !last.is_empty() && last.iter().all(|v| near(*v, target)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    panic!("koşul gerçekleşmedi: {what} (beklenen {target}, son görülen {last:?})");
}

#[test]
fn devices_volume_default_and_streams_native() {
    devices_volume_default_and_streams(AudioBackend::Native);
}

#[test]
fn devices_volume_default_and_streams_wpctl() {
    devices_volume_default_and_streams(AudioBackend::Wpctl);
}

fn devices_volume_default_and_streams(backend: AudioBackend) {
    let Some(pw) = PrivatePipeWire::start() else { return };
    let (svc, events) = start(&pw.socket(), backend);

    let s = state(&events, "ilk okuma", |s| {
        s.available && s.sinks.len() == 1 && s.sources.len() == 1 && s.sinks[0].volume > 0.99
    });
    let sink = s.sinks[0].clone();
    let source = s.sources[0].clone();
    assert_eq!((sink.name.as_str(), sink.description.as_str()), ("test.sink.a", "Hoparlör"));
    assert_eq!(source.description, "Mikrofon");
    assert!(s.default_sink().is_none(), "oturum yöneticisi yok: varsayılan henüz yok");

    // Varsayılan (WirePlumber'ın yazdığı anahtar) metadata'dan okunur.
    pw.tool("pw-metadata", &["-n", "default", "0", "default.audio.sink", r#"{"name":"test.sink.a"}"#, "Spa:String:JSON"]);
    state(&events, "varsayılan çıkış", |s| s.default_sink().is_some_and(|d| d.id == sink.id));

    // Dışarıdan (pavucontrol gibi) değişim: durum + OSD.
    pw.tool("pw-cli", &["set-param", &sink.id.to_string(), "Props", "{ channelVolumes: [ 0.125, 0.125 ] }"]);
    let osd = match events.wait_for("dış değişim OSD", |e| matches!(e, AudioEvent::Osd(_))) {
        AudioEvent::Osd(o) => o,
        _ => unreachable!(),
    };
    assert_eq!(osd.kind, OsdKind::Output);
    assert!(near(f64::from(osd.volume), 0.5) && !osd.muted, "{osd:?}");

    // Servisten yazma: algısal 0.8 → doğrusal 0.512.
    svc.set_volume(AudioTarget::DefaultSink, 0.8);
    volumes_reach(&pw, sink.id, "seviye yazıldı", 0.512);
    events.wait_for("OSD 0.8", |e| matches!(e, AudioEvent::Osd(o) if near(f64::from(o.volume), 0.8)));

    // Göreli: yankıyı beklemeden birikir, üst sınırda durur.
    for _ in 0..5 {
        svc.adjust_volume(AudioTarget::DefaultSink, 0.1);
    }
    volumes_reach(&pw, sink.id, "üst sınır", 1.0);
    svc.adjust_volume(AudioTarget::Node(sink.id), -0.5);
    volumes_reach(&pw, sink.id, "azaltma", 0.125);

    svc.toggle_muted(AudioTarget::DefaultSink);
    eventually("sessiz", || pw.props(sink.id)["mute"] == serde_json::Value::Bool(true));
    events.wait_for("sessiz OSD", |e| matches!(e, AudioEvent::Osd(o) if o.muted));
    svc.set_muted(AudioTarget::DefaultSink, false);
    eventually("ses açık", || pw.props(sink.id)["mute"] == serde_json::Value::Bool(false));

    // Geçersiz değerler ve bilinmeyen hedef.
    svc.set_volume(AudioTarget::DefaultSink, f32::NAN);
    events.wait_for("NaN", |e| matches!(e, AudioEvent::Failed { action: "volume", .. }));
    svc.set_volume(AudioTarget::Node(99_999), 0.5);
    events.wait_for("bilinmeyen", |e| matches!(e, AudioEvent::Failed { action: "volume", .. }));

    // Varsayılan seçimi: WirePlumber'ın okuduğu "configured" anahtarı.
    svc.set_default(source.id);
    eventually("configured source", || {
        pw.metadata().lines().any(|l| l.contains("default.configured.audio.source") && l.contains("test.source.a"))
    });

    // Uygulama akışı.
    pw.tool(
        "pw-cli",
        &[
            "create-node",
            "adapter",
            "{ factory.name = support.null-audio-sink node.name = app.out media.class = Stream/Output/Audio \
             application.name = Oyuncu media.name = \"Şarkı\" object.linger = true audio.position = [ FL FR ] }",
        ],
    );
    let s = state(&events, "akış", |s| s.playback.iter().any(|p| p.app == "Oyuncu" && p.volume > 0.99));
    let stream = s.playback.iter().find(|p| p.app == "Oyuncu").unwrap().clone();
    assert_eq!(stream.media.as_deref(), Some("Şarkı"));
    assert!(!stream.running, "bağlanmamış akış akmıyor (karıştırıcı göstermez)");
    svc.set_volume(AudioTarget::Node(stream.id), 0.5);
    volumes_reach(&pw, stream.id, "akış seviyesi", 0.125);
    pw.tool("pw-cli", &["destroy", &stream.id.to_string()]);
    state(&events, "akış gitti", |s| s.playback.is_empty());

    // Akan akış (kendi sürücüsü, sürekli işler): düğüm durumu `running` olarak gelir.
    pw.tool(
        "pw-cli",
        &[
            "create-node",
            "adapter",
            "{ factory.name = support.null-audio-sink node.name = app.run media.class = Stream/Output/Audio \
             application.name = Akan object.linger = true audio.position = [ FL FR ] \
             node.driver = true node.always-process = true }",
        ],
    );
    let s = state(&events, "akan akış", |s| s.playback.iter().any(|p| p.app == "Akan"));
    let id = s.playback.iter().find(|p| p.app == "Akan").unwrap().id;
    pw.tool(
        "pw-cli",
        &[
            "set-param",
            &id.to_string(),
            "PortConfig",
            "{ direction: Input, mode: dsp, format: { mediaType: audio, mediaSubtype: raw, format: F32P, \
             rate: 48000, channels: 2, position: [ FL FR ] } }",
        ],
    );
    state(&events, "akış running", |s| s.playback.iter().any(|p| p.id == id && p.running));
    // Karıştırıcının yazımı: akan akışın seviyesi ve sessizi (düğüm kimliğiyle). Yalnızca yerli
    // yolda: WirePlumber'sız bu daemon'da `wpctl` akan (kendi sürücülü) düğüme yazamıyor — gerçek
    // oturumda wpctl WirePlumber'ın mixer-api'sini kullanır; o yol ses kartıyla aynıdır.
    if backend == AudioBackend::Wpctl {
        drop(svc);
        return;
    }
    svc.set_volume(AudioTarget::Node(id), 0.5);
    volumes_reach(&pw, id, "akan akış seviyesi", 0.125);
    svc.toggle_muted(AudioTarget::Node(id));
    eventually("akan akış sessiz", || pw.props(id)["mute"].as_bool() == Some(true));
    drop(svc);
}

#[test]
fn reconnects_after_restart() {
    let Some(mut pw) = PrivatePipeWire::start() else { return };
    let (_svc, events) = start(&pw.socket(), AudioBackend::Native);
    state(&events, "ilk okuma", |s| s.available && s.sinks.len() == 1);
    pw.kill();
    state(&events, "PipeWire yok", |s| !s.available && s.sinks.is_empty());
    pw.spawn();
    state(&events, "geri geldi", |s| s.available && s.sinks.len() == 1 && s.sources.len() == 1);
}

#[test]
fn starts_without_pipewire() {
    let dir = tempfile::Builder::new().prefix("nxpw").tempdir_in("/tmp").unwrap();
    let (svc, events) = start(&dir.path().join("pipewire-0"), AudioBackend::Native);
    // Bağlanamaz, panik etmez, temiz kapanır.
    let quiet = events.collect(Duration::from_millis(300));
    assert!(quiet.iter().all(|e| matches!(e, AudioEvent::State(s) if !s.available)), "{quiet:?}");
    let t = Instant::now();
    drop(svc);
    // Güvenlik ağı: kapanış takılmamalı (yük altında planlama gecikmesi payı).
    assert!(t.elapsed() < Duration::from_secs(10), "{:?}", t.elapsed());
}

// ---------------------------------------------------------------------------------------
// WirePlumber ile karşılaştırmalı (diferansiyel) test: aynı işlem yerli yolla ve `wpctl`
// (WirePlumber mixer-api) ile yapılır, PipeWire'daki ham sonuç karşılaştırılır.

/// WirePlumber profili: donanım izleyicileri (ALSA/BlueZ/V4L2/libcamera) KAPALI — gerçek ses
/// kartına ve kullanıcının karıştırıcısına asla dokunulmaz; yalnızca politika ve metadata.
const WP_CONFIG: &str = r#"
wireplumber.profiles = {
  nuxut-test = {
    inherits = [ base, mixin.systemwide-session ]
    metadata.sm-settings = required
    metadata.sm-objects = required
    policy.standard = required
    hardware.audio = disabled
    hardware.bluetooth = disabled
    hardware.video-capture = disabled
    monitor.alsa = disabled
    monitor.alsa-midi = disabled
    monitor.bluez = disabled
    monitor.bluez-midi = disabled
    monitor.v4l2 = disabled
    monitor.libcamera = disabled
  }
}
"#;

/// Özel PipeWire'a bağlı özel WirePlumber (kendi config/state dizinleri, kendi D-Bus'ı).
struct PrivateWirePlumber {
    child: Child,
}

impl PrivateWirePlumber {
    /// Çalıştırılamazsa panikler; yalnızca `NUXUT_SKIP_PIPEWIRE_TESTS=1` ise `None` (atla).
    fn start(pw: &PrivatePipeWire, bus: &PrivateBus) -> Option<Self> {
        if let Err(e) = Command::new("wireplumber").arg("--version").stdout(Stdio::null()).status() {
            return skip::skip_or_panic("wireplumber", skip::SKIP_PIPEWIRE, e);
        }
        let d = pw.dir.path();
        let conf = d.join("config/wireplumber/wireplumber.conf.d");
        std::fs::create_dir_all(&conf).unwrap();
        std::fs::write(conf.join("90-nuxut-test.conf"), WP_CONFIG).unwrap();
        let child = Command::new("wireplumber")
            .args(["-p", "nuxut-test"])
            .env("XDG_RUNTIME_DIR", d)
            .env("PIPEWIRE_RUNTIME_DIR", d)
            .env("XDG_CONFIG_HOME", d.join("config"))
            .env("XDG_STATE_HOME", d.join("state"))
            .env("XDG_DATA_HOME", d.join("data"))
            .env("DBUS_SESSION_BUS_ADDRESS", &bus.address)
            .env_remove("PIPEWIRE_REMOTE")
            .env_remove("DBUS_SYSTEM_BUS_ADDRESS")
            .env("WIREPLUMBER_DEBUG", "1")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Some(Self { child })
    }
}

impl Drop for PrivateWirePlumber {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Testin kullanıcının ses sunucusuna gitmediğini doğrular: soket kullanıcının çalışma
/// dizininde ya da kullanıcının `PIPEWIRE_REMOTE`'u ise test çıkar.
fn isolated(pw: &PrivatePipeWire) -> bool {
    let socket = pw.socket();
    let user_runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    let user_remote = std::env::var_os("PIPEWIRE_REMOTE").map(PathBuf::from);
    let clash = user_runtime.as_deref().is_some_and(|r| socket.starts_with(r) || pw.dir.path() == r)
        || user_remote.as_deref().is_some_and(|r| r == socket);
    if clash {
        eprintln!("özel PipeWire kullanıcınınkiyle çakışıyor; test atlandı");
    }
    !clash
}

fn wpctl(pw: &PrivatePipeWire, args: &[&str]) -> String {
    pw.tool("wpctl", args)
}

/// `wpctl get-volume` → (seviye, sessiz).
fn wpctl_volume(pw: &PrivatePipeWire, target: &str) -> (f64, bool) {
    let out = wpctl(pw, &["get-volume", target]);
    let rest = out.trim().strip_prefix("Volume:").unwrap_or_else(|| panic!("beklenmeyen wpctl çıktısı: {out}"));
    let mut parts = rest.split_whitespace();
    let v: f64 = parts.next().unwrap().parse().unwrap();
    (v, parts.any(|p| p == "[MUTED]"))
}

fn diagnose(svc: &AudioService) -> String {
    let (tx, rx) = std::sync::mpsc::channel();
    assert!(svc.diagnose(Box::new(move |t| {
        let _ = tx.send(t);
    })));
    rx.recv_timeout(WAIT).expect("tanı raporu gelmedi")
}

#[test]
fn native_matches_wireplumber_mixer_on_props_path() {
    let Some(pw) = PrivatePipeWire::start() else { return };
    if !isolated(&pw) {
        return;
    }
    let Some(bus) = PrivateBus::start() else { return };
    let Some(_wp) = PrivateWirePlumber::start(&pw, &bus) else { return };
    // WirePlumber varsayılanı seçer (default.audio.sink).
    eventually("WirePlumber varsayılan çıkışı", || pw.metadata().contains("default.audio.sink"));
    let (sink, events) = channel();
    let options = AudioOptions {
        remote: Some(pw.socket().to_string_lossy().into_owned()),
        max_volume: 1.5,
        backend: AudioBackend::Native,
        coalesce: Duration::from_millis(10),
        max_backoff: Duration::from_millis(200),
    };
    let svc = AudioService::spawn(options, sink).unwrap();
    let s = state(&events, "varsayılan çıkış", |s| s.default_sink().is_some_and(|d| d.volume > 0.0));
    let id = s.default_sink().unwrap().id;
    let target = "@DEFAULT_AUDIO_SINK@";

    // 1) Mutlak seviyeler: yerli yazım ile wpctl yazımı PipeWire'da aynı ham değeri üretir ve
    //    wpctl yerli yazımı aynı yüzdeyle okur. %100 = doğrusal 1.0 (0 dB); %125 yükseltme.
    for level in [0.3f32, 0.75, 1.0, 0.0, 1.25] {
        svc.set_volume(AudioTarget::DefaultSink, level);
        eventually("wpctl yerli yazımı okur", || near(wpctl_volume(&pw, target).0, f64::from(level)));
        let native = pw.volumes(id);
        wpctl(&pw, &["set-volume", target, "0.42"]);
        state(&events, "wpctl 0.42", |s| s.default_sink().is_some_and(|d| near(f64::from(d.volume), 0.42)));
        wpctl(&pw, &["set-volume", target, &format!("{level}")]);
        let lvl = f64::from(level);
        let s = state(&events, "wpctl yazımı okundu", |s| s.default_sink().is_some_and(|d| near(f64::from(d.volume), lvl)));
        let theirs = pw.volumes(id);
        assert_eq!(native.len(), theirs.len());
        assert!(native.iter().zip(&theirs).all(|(a, b)| (a - b).abs() < 1e-5), "seviye {level}: yerli {native:?} ≠ wpctl {theirs:?}");
        assert!(native.iter().all(|v| (v - lvl.powi(3)).abs() < 1e-4), "kübik: {native:?}");
        assert!(near(f64::from(s.default_sink().unwrap().volume), lvl));
    }

    // 2) Sessiz: iki yönde.
    svc.set_muted(AudioTarget::DefaultSink, true);
    eventually("wpctl sessiz görür", || wpctl_volume(&pw, target).1);
    wpctl(&pw, &["set-mute", target, "0"]);
    state(&events, "wpctl sesi açtı", |s| s.default_sink().is_some_and(|d| !d.muted));

    // 3) Kanal dengesi: yerli yol oranı korur (pipewire-pulse/pavucontrol gibi); wpctl
    //    set-volume tüm kanalları eşitler (bilinçli fark).
    pw.tool("pw-cli", &["set-param", &id.to_string(), "Props", "{ channelVolumes: [ 0.5, 0.25 ] }"]);
    state(&events, "dengesiz", |s| s.default_sink().is_some_and(|d| near(f64::from(d.volume), 0.5f64.cbrt())));
    svc.set_volume(AudioTarget::DefaultSink, 0.8);
    eventually("denge korunur", || {
        let v = pw.volumes(id);
        v.len() == 2 && near(v[0], 0.512) && near(v[1], 0.256)
    });
    assert!(near(wpctl_volume(&pw, target).0, 0.8), "wpctl ilk kanalı okur");
    wpctl(&pw, &["set-volume", target, "0.8"]);
    volumes_reach(&pw, id, "wpctl eşitler", 0.512);

    // 4) Kaydırma patlaması: 30 göreli adım; yalnızca son değer önemli, adım kaybı yok.
    svc.set_volume(AudioTarget::DefaultSink, 0.3);
    eventually("0.3", || near(wpctl_volume(&pw, target).0, 0.3));
    for _ in 0..30 {
        svc.adjust_volume(AudioTarget::DefaultSink, 0.01);
    }
    eventually("patlama sonucu", || near(wpctl_volume(&pw, target).0, 0.6));
    state(&events, "durum 0.6", |s| s.default_sink().is_some_and(|d| near(f64::from(d.volume), 0.6)));

    // 5) Tanı raporu: yol, neden ve wpctl karşılaştırması; ad yok.
    let report = diagnose(&svc);
    println!("{report}");
    assert!(report.contains("yazım yolu ([audio] backend): native"), "{report}");
    assert!(report.contains(&format!("varsayılan çıkış: düğüm {id} (Audio/Sink)")), "{report}");
    assert!(report.contains("yol: PROPS — düğümde device.id yok"), "{report}");
    assert!(report.contains("wpctl get-volume @DEFAULT_AUDIO_SINK@ (varsayılan çıkış): Volume: 0.60"), "{report}");
    assert!(report.contains("uyum: evet"), "{report}");
    assert!(!report.contains("test.sink.a") && !report.contains("Hoparlör"), "ad sızmamalı: {report}");
    drop(svc);
}
