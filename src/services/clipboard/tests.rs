//! Servis testleri: sahte kaynakla gizlilik kanıtları (hassas teklifte ve kilitliyken `read`
//! hiç çağrılmaz), okuma/sınır, seçim, disk ve thread yaşam döngüsü.

use std::sync::mpsc;

use super::*;

/// Konan seçim: (MIME, bayt) çiftleri.
type Selection = Vec<(String, Arc<[u8]>)>;

/// Sahte kaynak: tekliflerin içeriği önceden verilir; her `read` ve `set_selection` kaydedilir.
#[derive(Default)]
struct FakeSource {
    contents: Mutex<Vec<(OfferId, String, Vec<u8>)>>,
    reads: Mutex<Vec<(OfferId, String, usize)>>,
    selections: Mutex<Vec<Selection>>,
}

impl FakeSource {
    fn offer(&self, id: u64, mime: &str, bytes: &[u8]) {
        self.contents.lock().unwrap().push((OfferId(id), mime.to_owned(), bytes.to_vec()));
    }

    fn reads(&self) -> Vec<(OfferId, String, usize)> {
        self.reads.lock().unwrap().clone()
    }
}

impl ClipboardSource for FakeSource {
    fn read(&self, id: OfferId, mime: &str, max_bytes: usize, _timeout: Duration) -> Result<Zeroizing<Vec<u8>>, ReadError> {
        self.reads.lock().unwrap().push((id, mime.to_owned(), max_bytes));
        let contents = self.contents.lock().unwrap();
        let (_, _, bytes) = contents.iter().find(|(i, m, _)| *i == id && m == mime).ok_or(ReadError::Gone)?;
        if bytes.len() > max_bytes {
            return Err(ReadError::TooLarge);
        }
        Ok(Zeroizing::new(bytes.clone()))
    }

    fn set_selection(&self, data: Vec<(String, Arc<[u8]>)>) -> Result<(), SetError> {
        self.selections.lock().unwrap().push(data);
        Ok(())
    }
}

struct Harness {
    source: Arc<FakeSource>,
    gate: Arc<Gate>,
    worker: Worker,
    events: mpsc::Receiver<ClipboardEvent>,
    available: Arc<AtomicBool>,
}

fn harness(config: ClipboardConfig, persist_path: Option<PathBuf>) -> Harness {
    let source = Arc::new(FakeSource::default());
    let gate = Arc::new(Gate::default());
    let (tx, events) = mpsc::channel();
    let sink: Sink = Box::new(move |e| {
        let _ = tx.send(e);
    });
    // Kaynak izliyor (adaptör başlatmış gibi); durma testi kendisi kapatır.
    let available = Arc::new(AtomicBool::new(true));
    let worker = Worker::new(source.clone(), gate.clone(), available.clone(), ServiceOptions { config, persist_path }, sink);
    Harness { source, gate, worker, events, available }
}

fn offered(id: u64, mimes: &[&str]) -> Input {
    Input::Source(SourceEvent::Offered { id: OfferId(id), mimes: mimes.iter().map(|s| (*s).to_owned()).collect(), flags: filter::Flags::NONE })
}

impl Harness {
    fn feed(&mut self, inputs: Vec<Input>) {
        assert!(self.worker.handle(inputs));
    }

    fn texts(&self) -> Vec<String> {
        self.worker.history.iter().map(|i| i.content.as_text().unwrap_or("<img>").to_owned()).collect()
    }

    fn last_snapshot(&self) -> Option<Arc<ClipboardSnapshot>> {
        self.events.try_iter().map(|ClipboardEvent::Snapshot(s)| s).last()
    }
}

#[test]
fn text_offer_is_read_once_and_stored() {
    let mut h = harness(ClipboardConfig::default(), None);
    h.source.offer(1, "text/plain;charset=utf-8", b"merhaba");
    h.feed(vec![offered(1, &["text/html", "text/plain;charset=utf-8", "image/png"])]);
    assert_eq!(h.source.reads(), [(OfferId(1), "text/plain;charset=utf-8".to_owned(), history::MAX_TEXT_BYTES)]);
    assert_eq!(h.texts(), ["merhaba"]);
    let s = h.last_snapshot().expect("yayın");
    assert!(s.enabled && s.available);
    assert_eq!(s.items.len(), 1);
}

#[test]
fn sensitive_offer_is_never_read_and_history_kept() {
    let mut h = harness(ClipboardConfig::default(), None);
    h.source.offer(1, "text/plain", b"onceki");
    h.feed(vec![offered(1, &["text/plain"])]);
    let reads_before = h.source.reads().len();
    for (id, mimes) in [
        (2, vec!["text/plain", filter::PASSWORD_HINT]),
        (3, vec!["application/x-nuxut-secret", "text/plain"]),
        (4, vec!["text/plain", "x-Password"]),
    ] {
        h.source.offer(id, "text/plain", b"parola");
        h.feed(vec![offered(id, &mimes)]);
    }
    assert_eq!(h.source.reads().len(), reads_before, "hassas teklifte read hiç çağrılmadı");
    assert_eq!(h.texts(), ["onceki"], "önceki öğeler silinmez");
}

#[test]
fn locked_session_and_excluded_app_are_never_read() {
    let mut h = harness(ClipboardConfig::default(), None);
    h.source.offer(1, "text/plain", b"kilitli");
    h.gate.set(true, None);
    h.feed(vec![offered(1, &["text/plain"])]);
    assert!(h.source.reads().is_empty(), "kilitliyken read hiç çağrılmadı");
    assert!(h.worker.history.is_empty());

    h.gate.set(false, Some("org.keepassxc.KeePassXC"));
    h.source.offer(2, "text/plain", b"kasa");
    h.feed(vec![offered(2, &["text/plain"])]);
    assert!(h.source.reads().is_empty(), "hariç tutulan uygulama okunmadı");

    h.gate.set(false, Some("firefox"));
    h.source.offer(3, "text/plain", b"serbest");
    h.feed(vec![offered(3, &["text/plain"])]);
    assert_eq!(h.source.reads().len(), 1);
    assert_eq!(h.texts(), ["serbest"]);
}

#[test]
fn own_and_disabled_offers_are_not_read() {
    let mut h = harness(ClipboardConfig { enabled: false, ..ClipboardConfig::default() }, None);
    h.source.offer(1, "text/plain", b"x");
    h.feed(vec![offered(1, &["text/plain"])]);
    assert!(h.source.reads().is_empty());
    let mut h = harness(ClipboardConfig::default(), None);
    h.source.offer(2, "text/plain", b"x");
    h.feed(vec![Input::Source(SourceEvent::Offered { id: OfferId(2), mimes: vec!["text/plain".into()], flags: filter::Flags::OWN })]);
    assert!(h.source.reads().is_empty(), "kendi seçimimiz geri okunmaz");
    h.feed(vec![Input::Source(SourceEvent::Cleared)]);
    assert!(h.source.reads().is_empty());
}

fn offered_with(id: u64, mimes: &[&str], flags: filter::Flags) -> Input {
    Input::Source(SourceEvent::Offered { id: OfferId(id), mimes: mimes.iter().map(|s| (*s).to_owned()).collect(), flags })
}

#[test]
fn framework_flags_are_never_read() {
    let mut h = harness(ClipboardConfig::default(), None);
    h.source.offer(1, "text/plain", b"onceki");
    h.feed(vec![offered(1, &["text/plain"])]);
    let reads = h.source.reads().len();
    for (id, flags) in [
        (2, filter::Flags::OWN),
        (3, filter::Flags { sensitive: true, ..filter::Flags::NONE }),
        (4, filter::Flags { truncated: true, ..filter::Flags::NONE }),
    ] {
        h.source.offer(id, "text/plain", b"gizli");
        h.feed(vec![offered_with(id, &["text/plain"], flags)]);
    }
    assert_eq!(h.source.reads().len(), reads, "own/sensitive/truncated teklifte read hiç çağrılmadı");
    assert_eq!(h.texts(), ["onceki"]);
}

#[test]
fn picked_item_comes_back_as_own_and_is_not_duplicated() {
    let mut h = harness(ClipboardConfig::default(), None);
    h.source.offer(1, "text/plain", b"bir");
    h.feed(vec![offered(1, &["text/plain"])]);
    h.source.offer(2, "text/plain", b"iki");
    h.feed(vec![offered(2, &["text/plain"])]);
    let first = h.worker.history.iter().find(|i| i.content.as_text() == Some("bir")).unwrap().id;
    h.feed(vec![Input::Pick(first)]);
    assert_eq!(h.texts(), ["bir", "iki"]);
    // Compositor geri koyduğumuz seçimi bildirir: framework `own` işaretler, okunmaz.
    let reads = h.source.reads().len();
    h.feed(vec![offered_with(3, &["text/plain;charset=utf-8", "text/plain"], filter::Flags::OWN)]);
    assert_eq!(h.source.reads().len(), reads);
    assert_eq!(h.texts(), ["bir", "iki"]);
}

#[test]
fn bridged_x11_selection_enters_history_but_sensitive_rules_still_apply() {
    use super::x11::policy::BRIDGE_MIME;
    let mut h = harness(ClipboardConfig::default(), None);
    // VM köprüsünün X11'den getirdiği kendi seçimimiz: `own` ama işaretli → geçmişe girer.
    h.source.offer(1, "text/plain", b"ana-makine");
    h.feed(vec![offered_with(1, &["text/plain", BRIDGE_MIME], filter::Flags::OWN)]);
    assert_eq!(h.texts(), ["ana-makine"]);
    // İşaretsiz kendi seçimimiz yine okunmaz.
    let reads = h.source.reads().len();
    h.source.offer(2, "text/plain", b"x");
    h.feed(vec![offered_with(2, &["text/plain"], filter::Flags::OWN)]);
    assert_eq!(h.source.reads().len(), reads);
    // İşaret `own` dışındaki engelleri kaldırmaz.
    h.source.offer(3, "text/plain", b"gizli");
    h.feed(vec![offered_with(3, &["text/plain", BRIDGE_MIME], filter::Flags { own: true, sensitive: true, truncated: false })]);
    assert_eq!(h.source.reads().len(), reads);
    assert_eq!(h.texts(), ["ana-makine"]);
}

/// Köprünün getirdiği seçim (`own` + işaret) geçmişe yazılmadan önce her kapıdan geçer:
/// geçmiş kapalı, oturum kilitli, hariç tutulan uygulama, parola ipucu, kırpılmış liste.
#[test]
fn bridged_selection_passes_every_gate_before_history() {
    use super::x11::policy::BRIDGE_MIME;
    let bridged = |id| offered_with(id, &["text/plain", BRIDGE_MIME], filter::Flags::OWN);
    // Geçmiş kapalı (köprü yine de panoyu izletir): okunmaz.
    let mut h = harness(ClipboardConfig { enabled: false, ..ClipboardConfig::default() }, None);
    h.source.offer(1, "text/plain", b"x");
    h.feed(vec![bridged(1)]);
    assert!(h.source.reads().is_empty());
    assert!(h.texts().is_empty());

    let mut h = harness(ClipboardConfig::default(), None);
    for id in 1..=5 {
        h.source.offer(id, "text/plain", b"gizli");
    }
    h.gate.set(true, None);
    h.feed(vec![bridged(1)]);
    h.gate.set(false, Some("Bitwarden"));
    h.feed(vec![bridged(2)]);
    h.gate.set(false, Some("firefox"));
    h.feed(vec![offered_with(3, &["text/plain", filter::PASSWORD_HINT, BRIDGE_MIME], filter::Flags::OWN)]);
    h.feed(vec![offered_with(4, &["text/plain", BRIDGE_MIME], filter::Flags { own: true, sensitive: false, truncated: true })]);
    assert!(h.source.reads().is_empty(), "hiçbir kapıda okunmadı");
    h.feed(vec![bridged(5)]);
    assert_eq!(h.texts(), ["gizli"], "kapılar açıkken girer");
}

#[test]
fn gate_blocks_transfer_when_locked_or_excluded() {
    let gate = Gate::default();
    assert!(!gate.blocks_transfer());
    gate.set_exclude_apps(&["Bitwarden".to_owned()]);
    assert!(gate.set(false, Some("bitwarden")));
    assert!(gate.blocks_transfer() && !gate.locked());
    assert!(!gate.set(false, Some("bitwarden")), "değişmedi");
    gate.set(false, Some("firefox"));
    assert!(!gate.blocks_transfer());
    gate.set(true, Some("firefox"));
    assert!(gate.blocks_transfer() && gate.locked());
    // Hariç tutma listesi `set` ile silinmez.
    gate.set(false, Some("Bitwarden"));
    assert!(gate.blocks_transfer());
}

#[test]
fn stopped_source_publishes_unavailable_and_keeps_history() {
    let mut h = harness(ClipboardConfig::default(), None);
    h.source.offer(1, "text/plain", b"kalir");
    h.feed(vec![offered(1, &["text/plain"])]);
    assert!(h.last_snapshot().unwrap().available);
    // Adaptör `Stopped` görünce bayrağı kapatır ve servisi dürter.
    h.available.store(false, Ordering::Relaxed);
    h.feed(vec![Input::Available]);
    let s = h.last_snapshot().expect("durum değişince yayın");
    assert!(!s.available);
    assert_eq!(s.items.len(), 1, "geçmiş korunur");
    // Dürtü kuyruk doluyken atılmış olsa da sonraki grup durumu yayınlar.
    h.available.store(true, Ordering::Relaxed);
    h.feed(vec![Input::Source(SourceEvent::Cleared)]);
    assert!(h.last_snapshot().expect("yayın").available);
    // Değişiklik yoksa yayın yok.
    h.feed(vec![Input::Available]);
    assert!(h.last_snapshot().is_none());
}

#[test]
fn only_latest_offer_in_a_burst_is_read() {
    let mut h = harness(ClipboardConfig::default(), None);
    for id in 1..=3 {
        h.source.offer(id, "text/plain", format!("t{id}").as_bytes());
    }
    h.feed(vec![offered(1, &["text/plain"]), offered(2, &["text/plain"]), Input::Clear, offered(3, &["text/plain"])]);
    assert_eq!(h.source.reads().iter().map(|r| r.0).collect::<Vec<_>>(), [OfferId(3)]);
    assert_eq!(h.texts(), ["t3"]);
}

#[test]
fn oversized_invalid_and_blank_content_not_stored() {
    let mut h = harness(ClipboardConfig::default(), None);
    h.source.offer(1, "text/plain", &vec![b'a'; history::MAX_TEXT_BYTES + 1]);
    h.feed(vec![offered(1, &["text/plain"])]);
    h.source.offer(2, "text/plain", b"   \n");
    h.feed(vec![offered(2, &["text/plain"])]);
    h.source.offer(3, "text/plain", b"\xff\xfe");
    h.feed(vec![offered(3, &["text/plain"])]);
    h.source.offer(4, "image/png", b"GIF89a-sahte");
    h.feed(vec![offered(4, &["image/png"])]);
    assert!(h.worker.history.is_empty());
    // Görsel okumada sınır görsel sınırıdır.
    assert_eq!(h.source.reads().last().unwrap().2, history::MAX_IMAGE_BYTES);
    // Görseller kapalıyken görsel teklif okunmaz.
    let mut h = harness(ClipboardConfig { images: false, ..ClipboardConfig::default() }, None);
    h.feed(vec![offered(5, &["image/png"])]);
    assert!(h.source.reads().is_empty());
}

#[test]
fn pick_sets_selection_and_moves_to_front() {
    let mut h = harness(ClipboardConfig::default(), None);
    h.source.offer(1, "text/plain", b"ilk");
    h.feed(vec![offered(1, &["text/plain"])]);
    h.source.offer(2, "image/png", b"\x89PNG\r\n\x1a\nimg");
    h.feed(vec![offered(2, &["image/png"])]);
    let first = h.worker.history.iter().find(|i| !i.is_image()).unwrap().id;
    h.feed(vec![Input::Pick(first)]);
    let sel = h.source.selections.lock().unwrap().clone();
    assert_eq!(sel.len(), 1);
    assert!(sel[0].iter().any(|(m, b)| m == "text/plain;charset=utf-8" && &b[..] == b"ilk"));
    assert!(sel[0].iter().any(|(m, _)| m == "UTF8_STRING"));
    assert_eq!(h.texts(), ["ilk", "<img>"]);
    let img = h.worker.history.iter().find(|i| i.is_image()).unwrap().id;
    h.feed(vec![Input::Pick(img)]);
    let sel = h.source.selections.lock().unwrap().clone();
    assert_eq!(sel[1].len(), 1);
    assert_eq!(sel[1][0].0, "image/png");
    // Bilinmeyen kimlik: hiçbir şey.
    h.feed(vec![Input::Pick(9999)]);
    assert_eq!(h.source.selections.lock().unwrap().len(), 2);
}

#[test]
fn delete_clear_and_disable() {
    let mut h = harness(ClipboardConfig::default(), None);
    for id in 1..=3 {
        h.source.offer(id, "text/plain", format!("t{id}").as_bytes());
        h.feed(vec![offered(id, &["text/plain"])]);
    }
    let id = h.worker.history.iter().nth(1).unwrap().id;
    h.feed(vec![Input::Delete(id)]);
    assert_eq!(h.texts(), ["t3", "t1"]);
    h.feed(vec![Input::Clear]);
    assert!(h.worker.history.is_empty());
    h.source.offer(4, "text/plain", b"t4");
    h.feed(vec![offered(4, &["text/plain"])]);
    h.feed(vec![Input::Config(ClipboardConfig { enabled: false, ..ClipboardConfig::default() })]);
    assert!(h.worker.history.is_empty(), "kapatılınca bellek bırakılır");
    assert!(!h.last_snapshot().unwrap().enabled);
}

#[test]
fn persist_writes_coalesced_reloads_and_removes() {
    let dir = tempfile::tempdir().unwrap();
    let path = store::path_in(dir.path());
    let config = ClipboardConfig { persist: true, ..ClipboardConfig::default() };
    let mut h = harness(config.clone(), Some(path.clone()));
    h.source.offer(1, "text/plain", b"kalici");
    h.feed(vec![offered(1, &["text/plain"])]);
    assert!(!path.exists(), "yazım birleştirilir (hemen değil)");
    assert!(h.worker.dirty_at.is_some());
    h.worker.flush();
    assert!(path.exists());

    // Yeni servis dosyadan yükler.
    let mut h2 = harness(config.clone(), Some(path.clone()));
    h2.worker.load();
    assert_eq!(h2.texts(), ["kalici"]);

    // persist kapatılınca dosya silinir.
    h2.feed(vec![Input::Config(ClipboardConfig { persist: false, ..config.clone() })]);
    assert!(!path.exists());

    // Geçmişi temizle dosyayı da siler.
    h.worker.mark_dirty();
    h.worker.flush();
    assert!(path.exists());
    h.feed(vec![Input::Clear]);
    assert!(!path.exists());

    // Varsayılan (persist kapalı): hiç yazılmaz.
    let mut h3 = harness(ClipboardConfig::default(), Some(path.clone()));
    h3.source.offer(1, "text/plain", b"x");
    h3.feed(vec![offered(1, &["text/plain"])]);
    assert!(h3.worker.dirty_at.is_none());
    h3.worker.flush();
    assert!(!path.exists());

    // Bozuk dosya: boş geçmiş, panik yok.
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"bozuk").unwrap();
    let mut h4 = harness(config, Some(path));
    h4.worker.load();
    assert!(h4.worker.history.is_empty());
}

#[test]
fn list_json_v1() {
    let mut h = harness(ClipboardConfig::default(), None);
    h.source.offer(1, "text/plain", b"ilk satir\nikinci");
    h.feed(vec![offered(1, &["text/plain"])]);
    h.source.offer(2, "image/jpeg", &[0xFF, 0xD8, 0xFF, 0xE0, 1, 2]);
    h.feed(vec![offered(2, &["image/jpeg"])]);
    let s = h.last_snapshot().unwrap();
    let v: serde_json::Value = serde_json::from_str(&s.list_json(SystemTime::now())).unwrap();
    assert_eq!(v["v"], 1);
    assert_eq!(v["enabled"], true);
    assert_eq!(v["available"], true);
    let items = v["items"].as_array().unwrap();
    assert_eq!(items[0]["kind"], "image");
    assert_eq!(items[0]["mime"], "image/jpeg");
    assert_eq!(items[0]["len"], 6);
    assert!(items[0].get("preview").is_none(), "görsel baytı/önizlemesi yok");
    assert_eq!(items[1]["kind"], "text");
    assert_eq!(items[1]["preview"], "ilk satir…");
    assert_eq!(items[1]["len"], 16);
    assert!(items[1]["age_s"].as_u64().is_some());
    let empty: serde_json::Value = serde_json::from_str(&ClipboardSnapshot::default().list_json(SystemTime::now())).unwrap();
    assert_eq!(empty["available"], false);
    assert_eq!(empty["items"].as_array().unwrap().len(), 0);
}

#[test]
fn parse_commands() {
    assert_eq!(parse_command(&["list"]), Ok(ClipboardCommand::List));
    assert_eq!(parse_command(&["pick", "12"]), Ok(ClipboardCommand::Pick(12)));
    assert_eq!(parse_command(&["delete", "7"]), Ok(ClipboardCommand::Delete(7)));
    assert_eq!(parse_command(&["clear"]), Ok(ClipboardCommand::Clear));
    for bad in [&["pick"][..], &["pick", "0"], &["pick", "01"], &["pick", "-1"], &["pick", "1x"], &["pick", "99999999999999999999999"], &["list", "x"], &[]] {
        assert!(parse_command(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn config_validation_and_schema() {
    let mut c = ClipboardConfig { max_items: 5, ..ClipboardConfig::default() };
    let mut w = Vec::new();
    c.validate(&mut w);
    assert_eq!(c.max_items, 10);
    assert_eq!(w.len(), 1);
    let mut c = ClipboardConfig { max_items: 9000, exclude_apps: vec!["ok".into(), "".into(), "a\nb".into(), "x".repeat(300)], ..Default::default() };
    let mut w = Vec::new();
    c.validate(&mut w);
    assert_eq!(c.max_items, 500);
    assert_eq!(c.exclude_apps, ["ok"]);
    assert_eq!(w.len(), 2);
    let mut d = ClipboardConfig::default();
    let mut w = Vec::new();
    d.validate(&mut w);
    assert!(w.is_empty(), "varsayılan geçerli");
    let mut schema = Vec::new();
    ClipboardConfig::schema(&mut schema);
    let keys: Vec<_> = schema.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(
        keys,
        ["clipboard.enabled", "clipboard.max_items", "clipboard.images", "clipboard.persist", "clipboard.primary", "clipboard.exclude_apps", "clipboard.x11_bridge"]
    );
    let bridge = schema.last().unwrap();
    assert_eq!(bridge.options, ["auto", "on", "off"]);
    assert_eq!(bridge.default, crate::schema::SettingValue::Text("auto".into()));
    // Liste varsayılanı satır içi TOML dizisi (`custom`), config varsayılanıyla aynı.
    let apps = schema.iter().find(|s| s.key == "clipboard.exclude_apps").unwrap();
    assert_eq!(apps.kind, crate::schema::SettingKind::Custom);
    let crate::schema::SettingValue::List(toml::Value::Array(list)) = &apps.default else { panic!("liste değil") };
    let names: Vec<_> = list.iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(names, ClipboardConfig::default().exclude_apps);
}

#[test]
fn service_thread_lifecycle() {
    let source = Arc::new(FakeSource::default());
    source.offer(1, "text/plain", b"thread");
    let (sink, inbox) = channel();
    let (tx, rx) = mpsc::channel();
    let service = ClipboardService::spawn(
        inbox,
        source.clone(),
        Arc::new(Gate::default()),
        ServiceOptions { config: ClipboardConfig::default(), persist_path: None },
        move |e| {
            let _ = tx.send(e);
        },
    )
    .unwrap();
    // Canlılık sınırı (yük altında da geçer; doğruluk saate bağlı değil).
    const LIVENESS: Duration = Duration::from_secs(120);
    let ClipboardEvent::Snapshot(first) = rx.recv_timeout(LIVENESS).unwrap();
    assert!(first.items.is_empty());
    assert!(!first.available, "kaynak başlamadan izlenemiyor");
    sink.set_available(true);
    let ClipboardEvent::Snapshot(on) = rx.recv_timeout(LIVENESS).unwrap();
    assert!(on.available);
    sink.send(SourceEvent::Offered { id: OfferId(1), mimes: vec!["text/plain".into()], flags: filter::Flags::NONE });
    let ClipboardEvent::Snapshot(s) = rx.recv_timeout(LIVENESS).unwrap();
    assert_eq!(s.items.len(), 1);
    service.pick(s.items[0].id);
    let ClipboardEvent::Snapshot(_) = rx.recv_timeout(LIVENESS).unwrap();
    assert_eq!(source.selections.lock().unwrap().len(), 1);
    drop(service);
    // Thread durdu: olay ucu artık bir şey üretmez, panik yok.
    sink.send(SourceEvent::Cleared);
}
