| `src/services/removable_media/` | udisks2 çıkarılabilir birim izleyicisi (`model.rs` saf süzgeç/etiket), Aç/Çıkar eylemleri |
# nuxut-shell — Mimari

Hyprland için masaüstü kabuğu. `nuxut-framework` üzerine kuruludur (`../framework`, path
bağımlılığı). Shell yalnızca shell'e özgü mantığı içerir; genel bileşen gerekirse framework'e
eklenir (bkz. framework `docs/DESIGN.md` ve `docs/ARCHITECTURE.md`).

## Yüzeyler (her monitör için)

| Yüzey | Katman | Anchor | Exclusive | Input | Amaç |
|---|---|---|---|---|---|
| `nuxut` (ana) | Top | 4 kenar | -1 (yok say) | çerçeve şeritleri + açık paneller | Çerçeve, bar, paneller — **tek blob alanı** |
| `nuxut-exclusion` (kenar başına) | Bottom | tek kenar | kenar kalınlığı | boş | Pencereleri çerçevenin içine itmek |
| `nuxut-screenshot` (yalnızca `shot region` sürerken) | Overlay | 4 kenar | -1 | tüm yüzey; klavye `Exclusive` hedef çıkışta | Dondurulmuş kare üstünde bölge seçimi (bkz. "Ekran görüntüsü") |
| `nuxut-osk` (yalnızca ekran klavyesi açıkken, tek monitör) | Overlay | iki kip de sol üst + margin (yanaşık: iç alanın altı, ortalı), gölge paylı aynı boyut; `[osk] reserve_space` yanaşıkken: alt kenar | -1 (`[osk] reserve_space` ile yanaşıkken kendi yüksekliği) | klavye dikdörtgeni; klavye **yok** | Ekran klavyesi (bkz. "Ekran klavyesi"); sürüklerken `set_layer_placement` ile taşınır |

Neden böyle: layer-shell'de exclusive zone yalnızca bir kenara uygulanır. Tam ekran ana yüzey
çerçeve + panelleri tek SDF alanında çizer (panellerin çerçeveden **büyüyerek** çıkması ancak
böyle mümkün), alan rezervasyonunu ise görünmez dört ince yüzey yapar (caelestia
`modules/drawers/Exclusions.qml` ile aynı yaklaşım). Ana yüzeyin input bölgesi yalnızca görünür
şekillerdir; ekranın ortası tıklamaları pencerelere geçirir.

## Geometri

- Çerçeve kalınlığı kenar başına (`Insets`). Bar bir kenarda (varsayılan üst) ve o kenarın
  kalınlığı = bar yüksekliği. Yani **bar, çerçevenin kalın bir kenarıdır**, ayrı bir pencere değil.
- Köşeler tek kuralla (ADR-0008, framework `docs/decisions/0008-panel-corner-geometry.md`): çerçeveye
  ya da panele birleşen köşe içbükey `R` (temanın `blob.smoothing`, çerçevenin iç köşeleri dahil),
  boşluğa açılan köşe dışbükey `r` (`shape.radius_large`, `BlobField::corner_radius`). Shell
  panelleri yalnızca dikdörtgen + deform verir (`BlobShape::new_rect`); ayrı köşe ayarı yok.
- Paneller (`BlobShape`) bir kenara yapışık başlar (yükseklik 0, kenarın içinde) ve yay ile
  dışarı büyür.
- Hızlı hareket eden panel hız yönünde esner (`blob.deform`), durunca yay ile oturur.
- Tüm geometri değerleri `Animated` — config değişince kalınlık/kavis bile yumuşakça değişir.
- Tam ekran pencere olan monitörde çerçeve kalınlığı ve kavsi 0'a animasyonla iner, input
  bölgesi boşalır. Yalnızca gerçek tam ekran (`j/clients` `fullscreen == 2`); maximize (1)
  rezerve alana saygılıdır, bar görünür kalır.
- Input bölgesi blob'un gerçek şeklidir (framework `BlobField::input_region`, ADR-0008 H):
  çerçeve + açık yapraklar + birleşim filetoları; boşluğa açılan `r` köşelerinin dışı tıklamayı
  geçirir. Hedef (oturmuş) şekilden, yalnızca alan değişince hesaplanır (önbellek anahtarı alan
  eşitliği, `ShellLayout::input_field`); overlay ve popup dikdörtgenleri eklenir. Açılışta hedef
  şekil hemen geçerli (açılan yaprak animasyon sürerken de tıklama alır), kapanışta hemen kalkar
  (kapanan yaprak tıklama yutmaz); animasyon karelerinde yeniden hesap yok. Ölçüm (
  2026-10-16; izole sway 1920×1080, release, çerçeve + sağ tam boy panel aç/kapa ×6): 175–265 µs
  ve 48 dikdörtgen per yeniden hesap. Süre debug logunda `micros` alanıyla izlenir
  ("girdi bölgesi blob şeklinden yeniden hesaplandı").
- Odak kapma (Hyprland `hyprland_focus_grab_v1`): geçici yaprak açıkken grab tutulur; başka
  uygulamanın menüsü kapanır, dışarı tıklama `cleared` ile yaprağı kapatır. Ayrıntı
  `docs/design/panels.md` "Odak kapma"; sway'de istek gitmez.
- Klavye politikası:
  - **Tıklamayla açılan** yaprak/menü: Hyprland `OnDemand`'e geçişte odak vermediği için önce
    `Exclusive` istenir; odak gelince (`keyboard_focus(true)`, en geç 300 ms sonra yedek timer
    ile) `OnDemand`'e inilir. Dışarı tıklama alttaki pencereye geçer ve odağı alır →
    `keyboard_focus(false)` → yaprak kapanır. (Canlıda kendi test penceresiyle doğrulandı.)
  - **Komutla (bind) açılan** yaprak: işaretçi bir pencerenin üstündedir; `OnDemand`'e inilirse
    Hyprland odağı o pencereye geri verir ve yaprak hemen kapanırdı. Bu yüzden kapanana kadar
    `Exclusive` kalır; Escape, aynı komut ya da bar kapatır.
  - Escape önce açık overlay'i (tepsi menüsü), sonra yaprağı kapatır.
- Input bölgesi ve klavye modu `Screen::take_input_change` ile yalnızca değişince gönderilir.
  Her olay işleyicisinin sonunda tüm ekranlar eşlenir (`Shell::refresh_inputs`); overlay'ler
  (tepsi menüsü) ve bildirim popup'ları boyutlarını çizimde öğrendiği için `draw` sonunda
  `input_stale` ise sıfır süreli bir timer kurulur ve eşleme hemen sonraki turda yapılır (input
  bölgesi `draw` içinde değiştirilemez; en fazla bir kare gecikme). Klavye isteyen bir şey
  kalmayınca odak bayrağı sıfırlanır: kendi kendine kapanan menüden sonra da yeni açılış
  `Exclusive` ile odak alır.
- Panel içeriği her karede **hedef boyutta** yerleşir (`Sheet::content_rect`: derinlikte uzak
  kenara yapışık, kenar boyunca `BlobShape::visual_rect()`'in ortasında) ve görünen
  dikdörtgene kırpılır; açılış/yay taşması/esneme boyunca içerik yeniden yerleşmez (saat
  popout'u titremesi, 2026-10-13: `docs/design/panels.md` §4). Opaklık panel
  genişliği ~%75, derinliği ~%80'i geçince açılır. Kapanırken yay
  sıfırı aşarsa şekil bar şeridine gömülür (blob çökmesi); oturan panel alandan çıkarılır.
- Hasar takibi: çerçeve yayı sürerken veya boyut/tema/config/tam ekran değişince tüm yüzey;
  aksi halde panelin eski ∪ yeni `damage_rect`'i ve bar hasarı (`src/bar_damage.rs`): bar
  yerleşimi (şerit, widget/uç dikdörtgenleri) değiştiyse ya da vurgu/yaprak geçişinde tüm şerit
  (eski ∪ yeni); değilse yalnızca değişen/animasyondaki widget'ın dikdörtgeni (bar ekseninde
  yarım boşluk payı, şeride kırpılı). Saat tıkı 1920 px şerit yerine yalnızca saat (≈90×36 px).
  Renderer bölgeye kırparak blob zemini dahil her şeyi çizdiğinden yarı saydam bar'da artık piksel
  kalmaz. İşaretlenen ama görünür bölgesi olmayan değişiklik açık boş hasar verir (tam yüzeye
  düşmez).
- Namespace'ler: ana yüzey `nuxut`, exclusion yüzeyleri `nuxut-exclusion` (boyut = kalınlık;
  kalınlığı 0 olan kenar ve tam ekranda yüzey yok).
- Aktif workspace hapı yuva sırası cinsinden (`Animated<f32>`) yaylanır: geometri değişince
  yuvayla birlikte gelir, yalnızca aktif workspace değişince kayar.
- `shell.toml` canlı izlenir; kalınlık/kavis/bar boyu yayla geçer, exclusion yüzeyleri
  yaratılır/kaldırılır/güncellenir. Bozuk dosyada son geçerli config korunur.

## Servisler

`src/services/` — UI'dan bağımsız, test edilebilir, ayrı thread'de bloklayan IO yapar ve
`App::Message` ile ana döngüye olay gönderir.

- `hyprland`: socket1 (komut, `j/` JSON) + socket2 (olay akışı). Kurallar (eski projeden ders):
  `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE`, read timeout, EOF = cevap sonu,
  `activeWorkspace` camelCase, olay patlamasını birleştirme (coalesce), kopunca yeniden bağlanma,
  bloklayan okumayı soket kapatarak sonlandırma.
  Uygulama (`src/services/hyprland.rs`): `reader` thread socket2'yi sınırlı satırlarla
  (64 KiB) okur, ilgili olayları sınırlı kanala "kirli" sinyali olarak atar; `refresher` thread
  tek bir `[[BATCH]]j/monitors;j/workspaces;j/activewindow` isteğiyle (1 s zaman aşımı, 4 MiB
  sınır) anlık görüntü alır. Birleştirme: boştayken ilk olay gecikmesiz yeniler, patlamanın
  kalanı 35 ms sessizlikten sonra tek yenileme olur, sürekli akışta en geç 150 ms'de bir.
  Aynı anlık görüntü iki kez yayınlanmaz. Kopunca 250 ms → 5 s üstel bekleme ile yeniden
  bağlanılır. Komutlar yalnızca tipli API'den (`dispatch_workspace(WorkspaceTarget)`).
  Sorgu hatası (karar 2026-10-13): durum sorgusu başarısız olursa (meşgul
  compositor'da zaman aşımı, kopuk/bozuk cevap) bekleyen değişiklik düşmez; refresher
  100 ms'den başlayıp her hatada ikiye katlanan, 2 s tavanlı beklemeyle yeniden sorgular
  (`query_retry_min/max`), başarıda bekleme sıfırlanır ve son durum yeni olay beklemeden
  yayınlanır. İlk hata `warn`, ardışıklar `debug` loglanır. socket2 koparsa bekleyen deneme
  iptal edilir (yeniden bağlanmada ilk durum zaten sorgulanır). Hata yokken ek timer/iş yok;
  kapanış kanal kapanınca denemeyi beklemeden çıkar.
- `clock` (`src/services/clock.rs`): saf fonksiyonlar. `ClockFormat::parse` chrono
  `StrftimeItems` ile panik etmeden doğrular (geçersiz → config uyarısı + otomatik), en küçük
  birimi (saniye/dakika) çıkarır; `until_next_tick` bir sonraki sınıra kalan süreyi (+5 ms)
  verir. Runtime tek atımlık timer kurar → boşta yalnızca dakikada bir iş. Kalıp yazılmadıysa
  `ClockFormat::auto` (`Locale::format_hm`, `framework.toml [locale] hour_cycle`'a uyar);
  `time_locale()` = framework `nuxut::i18n::time_locale()` (tek kaynak). Karar: `docs/design/clock-locale.md`.
- `timezone` (`src/services/timezone.rs`): `org.freedesktop.timedate1` `PropertiesChanged(Timezone)`
  salt dinleyicisi (ayrı thread, etkinleştirmez, sahip doğrulamalı) → `app/clock.rs` 1,1 sn sonra
  (chrono `/etc/localtime` önbelleği) ekranları yeniden kurar, gece ışığı konumu/planı yeniden hesaplanır; yedek `msg clock refresh`.
- `removable_media` (`src/services/removable_media/`): udisks2 (`org.freedesktop.UDisks2`, sistem
  bus'ı) ObjectManager izleyicisi (ayrı thread, sahip doğrulamalı, yeniden bağlanır). Birim =
  `Filesystem`'lı `Block`, `HintIgnore`/`HintSystem` değil, sürücüsü `Removable`/`MediaRemovable`/
  `usb`/`sdio`. Sonradan takılan birim → `app/removable_media.rs` "Aç / Çıkar" popup'ı
  (`msg removable open|eject <id>`); açılışta takılı olanlar bildirilmez, sökülünce bildirim
  kapanır, kendiliğinden bağlama yok. Masaüstü örneği udisks'i `GetManagedObjects` ile
  etkinleştirir; gerçek sistem bus'ındaki test örneği yalnızca izler. Karar: `docs/design/removable-media.md`.
- `tray`: StatusNotifierWatcher (yoksa sahiplenir) / Host + DBusMenu; `TrayModel`.
- `notifications`: `org.freedesktop.Notifications` sunucusu (ad başkasındaysa shell
  bildirimsiz çalışır), güvenli markup, görseller arka planda çözülüp küçültülür;
  `NotificationCenter` (popup'lar, geçmiş, DND, süre dolumu). Görsel çözme havuzu çekirdeklerin
  yarısı (en çok 3 thread); kuyrukta bekleyen kodlanmış baytlar `budget::DecodeBudget` ile
  sınırlı (toplam 64 MiB, bildirim başına 16 MiB): sığmayan görsel düşer, bildirim görselsiz
  kalır, ayırma iş nasıl biterse bitsin geri verilir. Shell'in kendi bildirimleri
  (`notifications::internal::InternalNotice { summary, body, actions, popup, image }`; görsel `with_image`: tema adı
  ya da en uzun kenarı `IMAGE_MAX_SIDE` (256) içinde, bayt sayısı tutarlı RGBA8 pixmap — çağıran
  küçültür, uymayan atılır; ör. ekran görüntüsü önizlemesi) D-Bus'a gitmeden
  `NotificationCenter::add_internal` ile merkeze eklenir (sunucu çalışmasa da): `popup = false`
  sessizdir (popup/ses yok, merkezde okunmamış), `true` sıradan popup kurallarına uyar. Düğme
  bir `msg` komut satırına yönlenir (`InternalAction { label, command }`, ör.
  `config undo-migration`, `hypr apply-switch`); basılınca `ipc::parse` + `Shell::execute`,
  bildirim kapanır. Kimlikler `INTERNAL_ID_BASE` (0xF000_0000) ve üstü; sunucu bu aralığı
  vermez. Ana döngüde `Shell::notice`, döngü öncesi `Shell::post_notice`, arka plan
  thread'inden `ShellMessage::Notice`.
- `session`: kullanıcı bilgisi + logind eylemleri (kilit/çıkış oturum nesnesi üzerinden,
  askıya al/yeniden başlat/kapat Manager'dan; süreç çalıştırılmaz), `Can*` yetenekleri.
- `calendar`: `nuxut-calendard` istemcisi (`nuxut-calendar-ipc`); UI'nın gösterdiği 6 haftalık
  aralık, değişiklik sinyallerinde birleştirilmiş yeniden sorgu; daemon yoksa takvim boş.
  Taklit koruması: sinyal yalnızca `org.nuxut.Calendar1` adının güncel sahibinden (tekil ad,
  `GetNameOwner` + bus sürücüsünden gelen `NameOwnerChanged`) kabul edilir; içeriği okunmaz,
  yalnızca "yenile" tetikler — veri her zaman sorguyla adın sahibinden gelir.
- `sysmon` (`src/services/sysmon/`): sistem izleyicisi — CPU (toplam + çekirdek),
  bellek + takas, GPU (amdgpu `gpu_busy_percent`, `mem_info_vram_*`; intel/nvidia sysfs'te
  vermez → gösterilmez, NVML sonraki adım), CPU sıcaklığı (hwmon önceliği: k10temp/zenpower
  Tctl, coretemp Package, cpu_thermal; hwmon yoksa thermal bölgeleri), ağ hızı (yalnızca
  `/sys/class/net/<ad>/device` olan fiziksel arayüzler — VPN/köprü trafiği iki kez sayılmaz),
  disk G/Ç (yalnızca `/sys/block/<ad>/device` olan bütün diskler), kök doluluğu (`statvfs`,
  15 sn'de bir). Ölçüm olayı olmayan veriler olduğundan bu servis **bilinçli olarak** aralıkla
  örnekler (`[sysmon] interval`, 0,5..=10 sn, varsayılan 2): yalnızca bar'da `sysmon` widget'ı
  varken çalışır (yoksa thread yok). Sık okunan dosyalar bir kez açılır, her ölçümde `pread`
  ile 0'dan okunur (sınırlı tampon, en çok 1 MiB). **Ayrıntı kipi** (popout açık): süreç
  listesi (`/proc/<pid>/stat`, tüm makineye göre CPU payı, en çok `[sysmon] processes`),
  tüm sıcaklık sensörleri, yerel bağlama noktaları, yük, frekans; açılınca hemen ölçer,
  süreç payları için 0,5 sn sonra ikinci ölçüm. Okuma aygıtı uyandıracak yerler atlanır:
  `runtime_status = suspended` GPU okunmaz ("uykuda"), `drivetemp` sensörü hiç okunmaz.
  Ayrıştırıcılar (`parse.rs`) saf ve düşmanca girdi testli; görünen metinler `format.rs`
  (Türkçe: "%23", "5,1 GiB"). Her aralık ölçümü yayınlanır (geçmiş ilerler); ölçüm gelince yalnızca bar'daki
  izleyici widget'ı ve popout güncellenir (`Screen::sync_sysmon`); bar değeri yalnızca eşiği aşınca
  (±2 puan, ±2 °C, ağda %25 / 4 KiB/s) yeniden yazılır, etiket değişmedikçe kare yok. Uyarı
  (≥ %90, ≥ 85 °C; `ALERT_PERCENT`/`ALERT_CELSIUS`) framework `Threshold` ile: geri dönüş paylı
  (3 puan / 2 °C), renk eşiğin `Tint`'i (varsayılan `error` rolü). **Geçmiş** (`history.rs`):
  CPU, bellek ve ilk GPU kullanımının son 60 örneği (sabit kapasiteli kuyruk, NaN = boşluk,
  `seq` sayacı) servis thread'inde, panel kapalıyken de birikir; yalnızca aralık ölçümleri iter
  (ayrıntı kipi ara ölçümü itmez) ve her ölçümle `Snapshot::history` içinde yayınlanır.
- `idle` (`src/services/idle/`, t061; tasarım `docs/design/idle.md`): boşta kalma —
  karart, ekranı kapat (Hyprland `dispatch dpms`), kilitle (logind `Session.Lock`), uyu
  (logind `Suspend`, varsayılan yalnızca pilde). **Hiçbir eylem varsayılan açık değil**
  (`[idle] dim/screen_off/lock/suspend = false`). `machine.rs` saf durum makinesi (aşama
  bildirimi, engeller manual/media/logind-idle, `LockedHint`, AC, `PrepareForSleep` →
  `Effect` farkları; rastgele dizi testli). `logind.rs` ayrı servis thread'i: `BlockInhibited`,
  `LockedHint`, `InhibitDelayMaxUSec`, `PrepareForSleep` (yalnızca login1 sahibinden), uyku
  gecikme inhibitörü (yalnızca `lock` açıkken; kilit kurulunca ya da en çok 30 sn sonra
  bırakılır), `SetIdleHint`. Boşta bildirimi compositor'ın ext-idle-notify-v1'inden
  (framework `cx.idle_watch`; aşama başına bir izleme, `watches.rs` kimlik↔aşama eşlemesi;
  destek yoksa `msg idle status` `source: unsupported`, sessiz yedek yok). Uygulama bağlantısı
  `app/idle.rs` (karartma örtüsü Overlay katmanı, boş input bölgesi). Test örneği gerçek
  logind'e bağlanmaz. `msg idle status|inhibit|uninhibit|toggle`.
- `screenshot` (`src/services/screenshot/`; tasarım `docs/design/screenshot.md`):
  `grim` alt süreci (PPM, bellekte çözülür), `geometry.rs` kesim/birleştirme, `selection.rs`
  bölge seçimi durum makinesi, `save.rs` PNG (`png`, hızlı sıkıştırma) + `O_EXCL|O_NOFOLLOW`
  kayıt + `wl-copy`, `hypr.rs` pencere geometrisi. Bağlantı `app/screenshot.rs`, katman
  `screenshot_view.rs`. Bkz. "Ekran görüntüsü".
- `clipboard` (`src/services/clipboard/`; tasarım `docs/design/clipboard.md`): pano
  geçmişi. `ClipboardSource` trait'i, `source.rs` framework `DataControl` adaptörü + `Restart`
  geri çekilmesi (compositor sunmuyorsa `available: false`), `filter.rs` gizlilik kararı (saf; MIME'a
  bakılır, hassas teklif ve kilitli oturumda **okunmaz**), `history.rs` sınırlar/tekilleştirme/
  sıfırlayarak silme, `store.rs` isteğe bağlı disk biçimi. Tek thread `nuxut-clipboard`; bağlantı
  `app/clipboard_history.rs` (kilit + etkin pencere sınıfı `Gate` ile), arayüz başlatıcının pano
  kipi. Bkz. "Pano geçmişi".
- Sistem servisleri ortak sözleşme: `XService::spawn(XOptions, sink)`, düşürülünce
  temiz kapanır; durum olayı tam anlık görüntüdür ve yalnızca değişince yayınlanır; polling yok
  (D-Bus sinyali, PipeWire olayı, sysfs `POLLPRI`); D-Bus sinyalleri yalnızca adın güncel
  benzersiz sahibinden kabul edilir, ad düşüp gelince durum sıfırlanıp yeniden okunur; bus
  koparsa üstel beklemeyle yeniden bağlanılır. Ortak altyapı `crates/nuxut-dbus` (sınırlı string/dizi
  okuyucular, `ObjectStore`, `ServiceThread`). Testler özel `dbus-daemon`/PipeWire üzerinde.
- `audio` (PipeWire, pipewire-rs): sink/source, varsayılan (`default` metadata), seviye/sessiz,
  uygulama akışları (`AudioStream::running`: düğüm durumu `running` — karıştırıcı yalnız çalanları gösterir;
  ham `pw_node_info.state` okunur, `NodeInfoRef::state()` UTF-8 dışı hata metninde panikler). Olaylar `State`, `Osd` (yalnızca aynı varsayılan cihazda gerçek değişim),
  `Failed`. Komutlar `set_volume`/`adjust_volume`/`set_muted`/`toggle_muted`
  (`AudioTarget::DefaultSink|DefaultSource|Node`), `set_default`. Seviye algısal (kübik),
  üst sınır `[audio] max_volume` (100..=150). Kart düğümünde cihaz `Route`'una, diğerlerinde
  `Props`'a yazılır. PipeWire yoksa `available = false`.
- `network` (`crates/nuxut-nm`; NetworkManager, sistem bus): durum, birincil bağlantı, Wi-Fi, erişim noktaları
  (SSID başına en güçlü), kayıtlı bağlantılar, VPN. Olaylar `State`, `SecretRequest`/`SecretDone`
  (şifre ajanı; yalnızca NM'den, şifre zeroize), `Failed` (`NmError`; metin kabukta). Komutlar `set_wifi_enabled`, `scan`
  (yalnızca panel açıkken), `connect_access_point`, `activate`, `deactivate`, `forget`,
  `provide_secret`. 802.1X ağlar ayarlara yönlendirilir. NM yoksa `available = false`.
  Şifre ajanı kimliği `org.nuxut.shell`; NM'de kimlik kullanıcı (UID) başına tektir ve sistem
  bus'ı `dbus-run-session` ile yalıtılmaz, bu yüzden test örneği (`NUXUT_SHELL_INSTANCE`) NM gizli bilgi ajanı
  kaydetmez (NM gizli bilgi kaydı yalnızca `NUXUT_TEST_NM_AGENT=1` ile kendi kimliğiyle `org.nuxut.shell.<ad>`).
  Kimlik başka süreçte kayıtlıysa bir kez uyarı; Wi-Fi paneli kullanılınca yeniden denenir.
  NM ajan kimliği seçimi (`agent_id_for`, `NUXUT_TEST_NM_AGENT`) kabukta (`src/services/network.rs`); crate
  yalnız `NetworkOptions::agent_id` alır (varsayılan `None`: gizli bilgi ajanı yok — Ayarlar; kabuk `with_agent`). Aşama 2 yazma API'leri
  (`connect_hidden`, `set_autoconnect`/`set_metered`, `import_wireguard`, `device_details`) ve olayları
  (`Done`, `DeviceDetails`, `ActivationFailed`) Ayarlar içindir; kabuk bu olayları yok sayar.
- `bluetooth` (`crates/nuxut-bluez`; BlueZ, sistem bus): adaptör, cihazlar (pil, RSSI). Olaylar `State`,
  `Pairing`/`PairingDone` (eşleştirme ajanı; yalnızca BlueZ'den), `Failed` (`BluezError`; metin kabukta). Komutlar
  `set_powered`, `start/stop_discovery`, `connect`, `disconnect`, `pair`, `forget`,
  `set_trusted`, `reply` (+ Ayarlar için `cancel_pairing`, adaptör/cihaz yazımları; tarama
  `SetDiscoveryFilter` Transport=auto + `own_discovery`; rfkill `RfkillBlocked`/`PowerState`/sysfs
  — network-settings.md §4). BlueZ yoksa `available = false`. Eşleştirme ajanı yolu `BluetoothOptions::agent_path`
  (crate varsayılanı `None` = BlueZ ajanı yok; kabuk `with_agent()` ile `AGENT_PATH` ister). `Pair` hataları tür
  (`AuthenticationRejected/Failed/Canceled/Timeout`, `ConnectionAttemptFailed` …); kabuk bunları
  `shell-bluetooth-error-call-failed` + D-Bus hata adıyla gösterir (önceki gibi genel satır).
  Sahte servisler: `crates/nuxut-{nm,bluez}/examples/fake_*` (yalnız `dbus-run-session`, oturum bus'ı;
  istemci `NUXUT_NM_BUS` / `NUXUT_BLUEZ_BUS=session` ile `bus_from_env()`; ikisi de yalnız
  `test-support` cargo özelliğiyle derlenir, üründe yok).
- `power` (`crates/nuxut-power`, Ayarlar ile paylaşılır; UPower + power-profiles-daemon, sistem
  bus): pil, şarj, kalan süre, AC, profil, pil şarj sınırı. Olaylar `Changed`, `BatteryWarning`
  (seviye başına bir kez), `ProfileDone { id, result }`, `ChargeLimitDone { id, result }`.
  Komutlar `set_profile -> id`, `set_charge_limit(bool) -> id`; ikisinde de yeni durum sonuçtan
  önce yayınlanır. Hata türü `PowerError` (ortak; metin kabukta `system.rs`). Test bus'ı
  `NUXUT_POWER_BUS=session` (`bus_from_env`, yalnız `test-support`).
  UPower yoksa `upower = false`, ppd yoksa `profiles = None`. Şarj sınırı birleşik
  `DisplayDevice`'ta değil gerçek pillerdedir: `EnumerateDevices` + `DeviceAdded`/`DeviceRemoved`
  ile `Type` = pil ve `PowerSupply` cihazları izlenir (pil başına seri sınırı); `charge_limit()` =
  destekleyen pillerin birleşimi (hepsi açıksa açık, karışık işaretli; eşikler yalnız hepsi
  aynıysa). İstekler sırayla işlenir; başarıda pil yeniden okunup yeni durum sonuçtan önce
  yayınlanır. Hatalar türlü: `Unavailable`, `Unsupported`, `Denied` (polkit
  `org.freedesktop.UPower.enable-charging-limit`), `Failed(<D-Bus hata adı>)`. Eşik değerleri
  salt okunur (udev hwdb `CHARGE_LIMIT`). Ayrıntı: `docs/design/power.md`.
- `brightness`: dahili panel (`/sys/class/backlight` okunur, logind `SetBrightness` ile yazılır)
  ve harici monitörler (DDC/CI, `/dev/i2c-N`, ayrı thread, istek birleştirme). Olaylar
  `Displays`, `Changed` (OSD), `Failed`. Komutlar `set`, `adjust`, `rescan`. Arka ışık ya da
  i2c izni yoksa ilgili ekran listede yok.
- `media` (MPRIS, oturum bus): oynatıcılar (aynı sahibin adları tekil, playerctld atlanır),
  metadata, konum tahmini. Olaylar `Players`, `Art`, `Failed`. Komutlar `action`
  (`MediaAction`), `refresh_position`, `fetch_art`: `file://` kapak `safe_file` ile, `https://`
  kapak `nuxut-net` `fetch_untrusted_media` ile (yalnızca genel internet, 8 MiB, 5 sn; reddedilen
  adreste UI yer tutucu); ikisinde de görsel imzası denetlenir. Oynatıcı yoksa liste boş.
- `launcher` (`src/services/launcher/`): uygulama başlatıcısı.
  - **Keşif** (`catalog.rs`, `desktop.rs`): `$XDG_DATA_HOME` + `$XDG_DATA_DIRS` altındaki
    `applications/` ağaçları; kimlik = göreli yol (`/` → `-`), ilk bulunan geçerli (gölgeleme),
    `Hidden=true` alttakini siler. Görünürlük: `Type=Application`, `NoDisplay`/`Hidden` değil,
    `OnlyShowIn`/`NotShowIn` × `$XDG_CURRENT_DESKTOP`, `TryExec` (mutlak ya da `$PATH`'te
    çalıştırılabilir), `Exec` ya da `DBusActivatable`. Yerelleştirilmiş `Name`/`GenericName`/
    `Comment`/`Keywords` (`lang_COUNTRY@MOD` → `lang`), `[Desktop Action]`'lar. Sınırlar: dosya
    64 KiB (O_NONBLOCK açılır, FIFO beklemez), değer 4 KiB, 16 eylem, derinlik 6, 20 000 dosya.
    Önbellek: (dev, ino, boyut, mtime) değişmeyen dosya yeniden okunmaz.
  - **Canlı güncelleme**: `nuxut-launcher-scan` thread'i inotify ile dizinleri (ve var olmayan
    dizinin en yakın üstünü) izler; patlama 200 ms sessizliğe (en geç 2 s) birleşir; yeni
    inotify nesnesi taramadan önce kurulur (değişiklik kaçmaz). Boşta poll'da uyur; durdurma eventfd.
  - **Arama** (`search.rs`): `nucleo-matcher`, UI thread'inde senkron; alanlar ad (×1.0),
    GenericName (×0.7), Keywords (×0.6), kimlik + program adı (×0.55); ad önek bonusu;
    frecency bonusu (`ln(1+n)` × yakınlık kovası, ≈ 2–3 karakterlik eşleşmeyle sınırlı).
    Boş sorgu: frecency, sonra alfabetik. 5000 uygulamada tuş başına ölçüm
    `examples/launcher_bench.rs`.
  - **Kullanım** (`usage.rs`): `$XDG_STATE_HOME/nuxut/launcher.toml`, ≤ 500 uygulama + ≤ 200 emoji
    (`[emoji]` tablosu; yalnız gömülü verideki diziler, boşsa yazılmaz) kaydı, 0600,
    geçici dosya + fsync + rename; bozuksa yok sayılır.
  - **Emoji** (`emoji.rs`, `data/emoji.tsv`, `docs/design/emoji.md`): 1914 taban
    emoji (Emoji ≤ 16.0, ten rengi varyantı yok), İngilizce + Türkçe CLDR adı/anahtar sözcüğü;
    `include_str!` ile gömülü (~249 KB), çalışma zamanında dosya okunmaz. `nuxut-launcher-run`
    açılışta bir kez ayrıştırır (UI thread'i beklemez). Arama: nucleo katlaması + Türkçe
    `I/İ/ı → i`, sözcük başı eşleşmesi, kademeler (ad tam > ad öneki > adda > anahtar sözcükte >
    alt dizgi); iki dil aranır, etkin olmayan dil puanı %80; kullanım kademe içinde öne alır.
    Veri `examples/emoji_import.rs` ile ibus'un CLDR türevi sözlüklerinden bir kez üretilir.
  - **Hesap makinesi** (`calc.rs`): saf Rust özyinelemeli iniş; girdi 128 bayt, derinlik 32,
    döngü yok; `+ - * / % ^ !`, ondalık virgül, birkaç fonksiyon/sabit. Tek sayı/sabit hesap sayılmaz.
  - **Başlatma** (`exec.rs`, `launch.rs`, `nuxut-launcher-run` thread'i): `Exec` spec'in tırnak
    kurallarıyla argv'ye çevrilir — **kabuk yok**; `%f %u %F %U` kaldırılır, `%i %c %k %%`
    açılır, bilinmeyen kod hata. `Terminal=true` → `[launcher].terminal` ya da ilk bulunan
    (`xdg-terminal-exec`, kitty, foot, alacritty, …). Süreç modeli:
    `systemd-run --user --scope --unit=app-nuxut-<kimlik>-<rastgele>.scope -- setsid -f <argv>`:
    uygulama kendi cgroup'unda ve oturumunda, shell'in çocuğu değil (ara zincirin çıkışı
    beklenir; kapsam açılamazsa yalnız `setsid -f`). Ortamdan `XDG_ACTIVATION_TOKEN`,
    `DESKTOP_STARTUP_ID`, `NUXUT_SHELL_INSTANCE` temizlenir; belirteç varsa ilk ikisi ona
    ayarlanır. `DBusActivatable` → `org.freedesktop.Application.Activate/ActivateAction`
    (`activation-token` platform verisi), başarısızsa `Exec`. Başarılı başlatma kullanıma
    yazılır. Loglarda ad/kimlik/argv/arama metni/belirteç yok.
  - **UI** (`src/launcher_view.rs`, `src/app/launcher.rs`): bar kenarının ortasından büyüyen
    yaprak (`PanelId::Launcher`; `nuxut-shell msg launcher toggle`, `launcher` bar widget'ı);
    arama alanı + framework `ResultList` (vurgu `match_chars`) + `→` ile masaüstü eylemleri alt
    sayfası. İkonlar arka planda (`nuxut_image::Loader`), görünen satırların çevresi için.
    Başlatma: `Action::Launch` → `request_activation_token` (son girdinin serial'i) →
    `App::activation_token` gelince istek servise (belirteç yoksa da başlatılır).
- `default_apps` (`src/services/default_apps/`; sözleşme defaults-v1,
  `docs/design/defaults.md`): `msg defaults status|set`. Dokuz rol (browser, email,
  file_manager, terminal, text_editor, image, video, music, pdf); aday ve "kurulu mu" launcher
  indeksinden (`Scanner::scan_index`: kimlik başına ilk girdi, `MimeType`/`Categories`).
  Mevcut seçim `mimeapps.list` zinciri (`<masaüstü>-` önce; config → config dirs → data home →
  data dirs; `[Removed Associations]` sonraki dosyalara) → `[Added Associations]` → `mimeinfo.cache`
  yedeği; yinelenen `[Default Applications]` bölümlü dosyaya yazılmaz; terminal
  `xdg-terminals.list` zinciri (`xdg-terminal-exec` PATH'te değilse `available:false`). Yazım
  `nuxut-apply` ile yalnızca `$XDG_CONFIG_HOME/mimeapps.list` `[Default Applications]`
  (`IniKey`; diğer her şey korunur, dosya yoksa yaratılır) ve `xdg-terminals.list` (`Line`,
  başa); snapshot `defaults.<rol>`. İstekler `nuxut-defaults` thread'inde sırayla (ilk komutta
  açılır; ayrıştırma önbelleği thread'de). Dosya izleme yok.
- Bağlantı: her servis ilk output'ta (run sonrası; sinyal maskesi) başlar, sink →
  `MessageSender<ShellMessage>`; başlamayan servis shell'i durdurmaz. Olaylar `ShellModel`'e
  uygulanır, değişen ekranlar `sync_model` ile güncellenir. Widget'lar servis tutmaz; eylemler
  (`Action::Notify/Tray/Session/CalendarRange`) `App` tarafından servis tutamaklarına iletilir.
- **Log politikası:** kullanıcı içeriği (bildirim metni, pencere başlığı, URL, IPC argümanları,
  etkinlik verisi) hiçbir log seviyesinde yazılmaz. Servis olay tiplerinin (`HyprlandEvent`,
  `NotifyEvent`, `TrayEvent`, `CalendarEvent`, `AudioEvent`, `NetworkEvent`, `BluetoothEvent`,
  `MediaEvent`) `Debug`'ı redaktedir (yalnızca kimlik/sayı; SSID, cihaz adı, medya başlığı yok);
  serde_json hataları yalnızca tür ve konumla yazılır (`json_error_summary`).
- **Görsel güveni** (framework `nuxut_image::Trust`, varsayılan `Untrusted`: SVG süreç içinde
  ayrıştırılmaz, yalnızca raster): başka süreçten gelen her görsel `Untrusted` — tepsi
  `IconThemePath`/mutlak yol/dbusmenu `icon-data`, bildirim `image-path`/dosya `app_icon`/
  `image-data`, MPRIS kapağı. `Trust::System` yalnızca sistem temasında doğrulanmış adla aranan
  ikona istenir (tepsi `IconSource::Theme`, bildirim tema adlı `app_icon`); framework bulunan
  dosyayı fd'den denetler, kullanıcının yazabildiği tema dizinindeyse yine `Untrusted`'a düşer.
  D-Bus pixmap'leri ham pikseldir, çözücüye girmez.
- **Görsel yalıtımı** (framework ADR-0007, `services/image_sandbox.rs`; `ENABLED = true`,
  install.sh `INSTALL_HELPERS=1`): açılışta tek `Arc<Sandbox>` (`SandboxConfig::default()`,
  tembel; boşta en çok 1 önceden ısıtılmış taze işçi) kurulur; tepsi, bildirim, MPRIS kapağı ve
  başlatıcı ikonu Loader'ları onu `with_sandbox` ile paylaşır (hız kovası, negatif önbellek,
  anahtar susturma ortak). Her `Untrusted` görsel — SVG dahil — ayrı, tek kullanımlık
  `nuxut-image-worker` sürecinde (seccomp + landlock + rlimit) çözülür. Anahtarlar
  (`source_key`: süreç başına rastgele anahtarlı SipHash, asla 0): tepside öğe kimliği, MPRIS'te
  oynatıcı bus adı (`load_keyed`: iptal = kaynak), başlatıcı anahtarsız. Bildirimde
  `load_with_keys` iki anahtarı ayırır: iptal = gönderen + yuva + bildirim kimliği (art arda
  gelen bildirimler birbirini iptal etmez; `replaces_id` eskisini iptal eder), kaynak = gönderen
  (`KeyDomain::NotifySender`; susturma/hız sınırı gönderen başına). Gönderen D-Bus'ın doğruladığı
  kimliktir (`notifications::sender`: benzersiz ad → `GetConnectionUnixProcessID` →
  `/proc/<pid>/exe` + cgroup v2 yolu; çözülemezse ortak `anon` kovası — fail-closed: kısa ömürlü bağlantılarla susturmadan kaçılamaz;
  bağlantı başına önbellek, 256 girdi); istemcinin `app_name`'i yalnızca görüntüdür — taklit eden başkasını susturtamaz. `Err` her zaman yedeğe
  düşer: tepside öğe pixmap'i/yer tutucu, bildirimde semantik ikon, kapakta yer tutucu. İşçi
  bulunamazsa (`Unavailable`) kapılar yine uygulanır, yalnızca PNG/JPEG/WebP/GIF/QOI süreç içinde
  ilk kare; SVG/JXL/AVIF yedekle (K2-b). İşçi framework'teki ayrı paket (shell `[[bin]]`
  tanımlamaz, JXL çözücüsü kabuk ikilisine girmez): `cargo build --release --manifest-path
  ../framework/Cargo.toml -p nuxut-image-worker`; install.sh `~/.local/lib/nuxut/` (0755) altına
  kurar. Geliştirmede `cargo run` işçiyi derlemez: `NUXUT_IMAGE_WORKER=<yol>` ya da kurulu işçi
  (sürümü uymazsa el sıkışma reddeder → `Unavailable`). Testler işçiyi `tests/support/sandbox.rs`
  ile `CARGO_TARGET_TMPDIR` altına bir kez derler. Tanı: `msg images status`.
- **Duvar kağıdı yalıtımı** (framework "Duvar kağıdı yalıtımı", ADR-0007 takip; karar 2026-10-13 —
  framework S3 kırıcı imzasının uyarlaması): duvar kağıdı kullanıcı dosyasıdır,
  her biçim (JXL dahil) güven ne olursa olsun `nuxut-image-worker`'da **duvar kağıdı profiliyle**
  çözülür. `image_sandbox::new_wallpaper_decoder()` açılışta süreç başına **tek**
  `Arc<WallpaperDecoder>` kurar (kendi `Sandbox::new(SandboxConfig::wallpaper())`'ı: ikon
  sandbox'ından ayrı havuz/kova/susturma — tepsi fırtınası duvar kağıdını hız sınırına sokamaz;
  varsayılan önbellek 96 MiB, `linger` 10 sn); bütün çıkışların `WallpaperSurface`'ları aynı
  `Arc`'ı `Wallpaper::new(fit, motion, decoder, notify)`'a verir. Aynı dosya + fiziksel boyut tek
  çözüm (birleştirme/önbellek), küçük çıkış büyük kareden türetilir (canlı: 1920×1080 + 1280×720
  → `decodes: 1, derived: 1`). Boşta thread/işçi yok. `ENABLED` kapısı duvar kağıdına uygulanmaz
  (sandbox'sız süreç içi yol yok; işçi yoksa framework S1: PNG/JPEG/WebP/GIF/QOI ilk kare süreç
  içinde, diğerleri `Unavailable`). **Animasyonlu kaynak** (GIF/WebP/APNG, S2-a): ilk
  kare durağan gösterilir, kare/timer yok (shell'deki animasyon timer'ı kaldırıldı); bir kez
  bilgi loglanır, hareketli duvar kağıdı `nuxut-video` içe aktarmasıyla video yoluna gider (ayarlar).
  **Hata** (`Unavailable`/`Protocol`/zaman aşımı/bozuk ya da çok büyük dosya): panik yok, önceki
  görsel kalır, hiç yoksa `[wallpaper] backdrop_color` zemini; hata metni çizimde bir kez
  `warn` (yol loglanmaz). Kaynak çözümlemesi tek noktadadır (`WallpaperSurface::apply_source`):
  dosya yolu ya da **yerleşik** `nuxut:<id>` (aşağıda "Yerleşik duvar kağıtları"). `Importer` (nuxut-video) shell'de
  kullanılmıyor. Tanı: `msg images status` → `wallpaper` nesnesi (`decodes`, `hits`, `joined`,
  `derived`, `cancelled`, `failed`, `resident_bytes`, `pending`).
- **Yerleşik duvar kağıtları** (karar 2026-10-13, kullanıcı kararı; ayrıntı
  `docs/design/wallpapers.md`): `crates/nuxut-wallpapers` dokuz tasarımın koyu/açık SVG'lerini
  derleme anında gömer (`pair-flight` = varsayılan, `wingtip`, `climb`, `migration`,
  `silhouette`, `aurora`; maskotlu `cloud-rest`, `far-glide`, `chase`). Kaynak `nuxut:<id>`
  tema kipini izler (`framework.toml [colors]` çözülmüş kipi; kip değişince — tema yeniden
  yüklemesiyle aynı an, `set_theme` — yeni varyant çapraz geçişle), `nuxut:<id>/dark|light`
  sabittir (eski `:dark` biçimi kabul edilmez); bilinmeyen kimlik/varyant config uyarısı +
  `nuxut:pair-flight`. Aynı adres `[wallpaper.outputs.<ad>] path`, `lock.toml wallpaper` ve
  `palette.toml [meta] image`'da (kilidin palet görseli yedeği) geçerlidir. Gömülü kaynak güvenilir:
  süreç içinde `resvg` ile çıkışın **tam fiziksel boyutunda** cover render + bant kırıcı sıralı
  dither; süreç başına tek `Renderer` (iş varken tek thread, aynı anahtar birleşir). Kare
  **bellekte** kalır ve bileşene framework `Wallpaper::set_image(image_key, Frame)` ile kopyasız
  verilir (paylaşılan `Arc<[u8]>`; anahtar = FNV(SVG ‖ kimlik ‖ `PIPELINE` [‖ renk teması])): disk önbelleği,
  PNG kodlama ve yalıtılmış çözüm turu yok (karar 2026-10-13; framework
  7375116). Kullanıcı dosyaları yine yalıtılmış `WallpaperDecoder`'dan geçer.
  Renk teması (`[colors] source = "static"`, `theme` ≠ blue): yerleşik SVG renkleri temaya
  taşınarak çizilir, tema değişince kip değişimi gibi çapraz geçiş; kullanıcı görselleri
  boyanmaz (2026-10-16, `docs/design/wallpapers.md` madde 13).
  Render hatası panik değil: zemin rengi + bir kez `warn`. Kilit ekranı aynı crate'i kullanır
  (aşağıda "Kilit ekranı"). Material You tohumu render'dan
  çıkarılmaz: `Builtin::seed(variant)` sabit tablosu (palet üretimi ayarlarda; shell'de palet
  çıkarımı yok). Tanı/ayarlar: `msg wallpaper builtins`.

## shell.toml şeması ve varsayılanlar

```toml
[frame]
enabled      = true     # false: yalnızca bar kenarı kalır
thickness    = 6.0      # tüm kenarlar
# top / right / bottom / left = ...   # kenar başına geçersiz kılma (bar kenarında yalnızca bar gizliyken)

[bar]
edge    = "top"         # "top" | "bottom" | "left" | "right" (sol/sağ = dikey bar)
size    = 34.0          # bar kalınlığı = o kenarın çerçeve kalınlığı
padding = 10.0          # bar boyunca uçlardaki iç boşluk
spacing = 12.0          # widget'lar arası
start   = ["logo", "workspaces"]                  # yatay: sol, dikey: üst
center  = ["clock", "next_event", "now_playing"]     # now_playing çalan yokken, next_event eşik içinde etkinlik yokken gizli (2026-10-15, 2026-10-16)
end     = ["privacy", "magnifier", "keyboard_layout", "sysmon", "tray", "status", "notifications", "session"]    # yatay: sağ, dikey: alt. privacy kullanım yokken, magnifier büyütme yokken, keyboard_layout tek düzende gizli (2026-10-16)
# widget'lar: workspaces, clock, active_window, now_playing, privacy, caffeine, keyboard_layout, next_event, magnifier (bar hapı + panel kartı), tray, session, notifications, panel-toggle, launcher, sysmon, status, logo

[bar.edge_actions]      # uç widget eylemleri (docs/design/bar-edge-actions.md)
scroll_step = 5         # çentik başına %, 1..=25
[bar.edge_actions.start]   # yatay: en sol, dikey: en üst widget; end: en sağ / en alt
left_click  = "widget"  # (end varsayılanı: "panel:right") widget | none | panel:left|right|top|bottom|clock|launcher | volume | mic (sessiz aç/kapa)
right_click = "widget"
scroll      = "brightness"  # (end: "volume"; parlaklık kontrolü yoksa yok sayılır, yer ayrılmaz) widget | none | volume | mic | brightness | workspace
                        # ikonlu kaydırma eyleminde widget'ın dış yanında ikon yuvası baştan ayrılır

[bar.widget.workspaces]
count  = 10             # sayfa başına nokta (1..=30); her zaman bu kadar yuva
style  = "icons"        # "icons" (uygulama glifi, yoksa numara) | "numbers" | "dots"
scroll = "all"          # "all" | "occupied" | "off" — tekerlek/dokunmatik yüzeyle geçiş
# tasarım: docs/design/workspaces.md

[bar.widget.clock]             # ipucu: gün adıyla tam tarih ("Salı, 13 Ekim 2026"; popout açıkken yok)
# yazılmamış ya da "" = otomatik: [locale] hour_cycle ("13:05" / "1:05 PM"; dikey "13\n05" / "1\n05\nPM")
format          = "%H:%M"     # yatay bar; yazılırsa her zaman kazanır (docs/design/clock-locale.md)
vertical_format = "%H\n%M"    # dikey bar (satırlara bölünür)

[bar.widget.active_window]
max_chars = 40          # en fazla grafem (8..=200); uzunsa sonu "…", tamamı ipucunda
max_width = 360.0       # piksel sınırı; dikey bar'da gizli

[bar.widget.tray]
icon_size = 18.0

[bar.widget.session]
panel = "right"         # açtığı kenar paneli

[bar.widget.notifications]
show_zero = false
panel     = "right"

[bar.widget.panel-toggle]
panel = "right"
icon  = "menu"          # semantik ikon adı (menu, settings, apps, tune …; nuxut::ui::IconName)

[bar.widget.sysmon]     # sistem izleyicisi: bar'da görünen ölçümler (ipucu ve popout hepsini gösterir)
cpu         = true
memory      = true
gpu         = false     # amdgpu kullanımı; GPU yoksa/uykudaysa gizli
temperature = false     # CPU sıcaklığı (sensör yoksa gizli)
network     = false     # ↓ ↑ hız
disk        = false     # kök doluluğu
# hiçbiri seçili değilse yalnızca ikon; tıklayınca ayrıntı popout'u

[bar.widget.now_playing] # şimdi çalıyor: etkin MPRIS oynatıcısı (medya kartıyla aynı model; docs/design/now-playing.md)
width  = 240.0          # yatay bar'da SABİT genişlik (120..=600): parça/durum değişince yerleşim kaymaz, yalnız
                        # widget dikdörtgeni yenilenir; metin "Başlık · Sanatçı", sığmazsa "…". Dikey bar: yalnız kapak
scroll = "volume"       # "volume" (ses ± edge_actions.scroll_step, OSD) | "track" (yukarı sonraki, aşağı önceki;
                        # hareket başına tek atlama) | "off"
panel  = "right"        # sol tık: açılan kenar paneli (medya kartı); orta tık: oynat/duraklat
# çalan ya da duraklatılmış oynatıcı yoksa gizli; ilerleme bar'da yok (saat vuruşu dinlenmez)
# ipucu: tam başlık, sanatçı, albüm, oynatıcı · durum; kapak: [media] remote_art kuralları

[bar.widget.caffeine]   # uyanık tut düğmesi: ShellModel::idle_inhibit (hızlı anahtar ve `msg idle inhibit` ile aynı
                        # elle engel); tık aç/kapa; açıkken primary dolgulu hap
show_when_off = true    # false: yalnız açıkken görünür (hatırlatıcı)

[bar.widget.privacy]    # gizlilik göstergesi (docs/design/privacy-indicator.md): yalnız kullanımdayken görünen dolgulu hap
microphone = true       # PipeWire kayıt akışı var (ses servisi)
camera     = true       # bir süreç /dev/video* açık tutuyor: services::camera (inotify IN_OPEN/CLOSE + yalnız
                        # readlink /proc/*/fd ve comm; boşta yoklama yok, açıkken 5 sn doğrulama; false ise izleyici hiç çalışmaz)
screen     = true       # ekran sürekli yakalanıyor (Hyprland screencast: portal paylaşımı, wf-recorder, OBS);
                        # ayrı kayıt göstergesi gerekmez. Ekran varsa hap error, yoksa primary dolgu
panel      = "right"    # tıklayınca açılan panel (ses kartı); ipucu: uygulamalar + yakalamanın başlangıcı/süresi

[bar.widget.status]     # durum göstergeleri (t062): ağ · Bluetooth · ses · pil ikon grubu
network         = true  # Wi-Fi gücü / kablolu / bağlantı yok (soluk); internet yoksa soluk + ipucunda neden
bluetooth       = true  # açık / bağlı; kapalıyken ya da adaptör yokken gizli
volume          = true  # seviye / sessiz (soluk); üzerinde tekerlek = ses ± edge_actions.scroll_step, OSD
battery         = true  # şarj / seviye; ≤ %15 ve şarjda değilken error rengi; pil yoksa gizli
volume_percent  = false # ikonun yanında yüzde
battery_percent = true
panel           = "right"  # tıklayınca açılan kenar paneli (hızlı ayar kartları)
# her göstergenin kendi ipucu (SSID+güç, cihaz, bağlı BT cihazları+pil, kalan süre);
# yalnızca görünen ikon/değer/ton değişince yeniden çizilir (servis olayı → sync)

[panels.right]          # kenar çekmecesi: çerçeve kenarından tam boy büyür
enabled = true
size    = 380.0         # derinlik (sol/sağ: genişlik, üst/alt: yükseklik)
widgets = ["session", "toggles", "volume", "mixer", "brightness", "network", "bluetooth", "power", "media", "notifications"]
                        # panelde: session (oturum kartı), notifications (merkez), hızlı ayarlar:
                        # toggles (hızlı anahtarlar: Wi-Fi, BT, RE, uyanık tut, gece ışığı, koyu tema; docs/design/quick-toggles.md),
                        # volume (2+ cihazda tek tıkla çıkış/giriş cihazı seçimi; docs/design/audio-devices.md), mixer (ses karıştırıcı: ses çalan uygulamalar; docs/design/mixer.md), brightness, network, bluetooth, power, media (bar'a konamaz; veri yoksa gizli),
                        # calendar (takvim kartı; bar'a konamaz — bar'daki takvim clock'un popout'u)

[panels.left]           # varsayılan kapalı; top/bottom aynı
enabled = false
size    = 340.0
widgets = []

[sysmon]                # örnekleme (servis yalnızca bar'da sysmon varken çalışır)
interval  = 2.0         # sn, 0.5..=10
processes = 5           # popout'ta en çok CPU kullanan süreç (0..=15; 0 = süreç listesi okunmaz)

[audio]
max_volume = 100        # ses üst sınırı, yüzde (100..=150; üstü yükseltme)

[media]
remote_art = true       # oynatıcının https kapağını indir; false: istek yok (gizlilik), yer tutucu

[notifications]         # Ayarlar ile kesinleşti (Ayarlar bu anahtarları yazar)
enabled             = true
dnd                 = false   # rahatsız etme: critical dışı popup yok, geçmişe yazılır
popup_timeout_ms    = 5000    # 1000..=60000
# critical_timeout_ms = 10000 # yoksa critical kendiliğinden kapanmaz
max_popups          = 4       # 1..=10
history_limit       = 100     # 0..=500

[launcher]              # uygulama başlatıcısı
enabled       = true
terminal      = []      # Terminal=true için önek argv, ör. ["kitty", "-e"]; boş: otomatik
systemd_scope = true    # her uygulama kendi systemd kapsamında (app-nuxut-*.scope)
calculator    = true    # ifade yazınca hesap satırı
width         = 560.0   # 360..=900
rows          = 8       # görünen sonuç satırı, 4..=16

[notifications.apps."<app_name>"]   # uygulama başına (app_name birebir, büyük/küçük harf duyarlı)
popups  = true          # false: popup yok, yalnızca merkez
history = true          # false: geçmişe girmez

[wallpaper]
path = "nuxut:pair-flight"  # varsayılan (yazılmamış = taze kurulum): yerleşik "Eş uçuş", tema kipini izler.
                          # "nuxut:<id>[/dark|light]" yerleşik (pair-flight|wingtip|climb|migration|silhouette|aurora|cloud-rest|far-glide|chase),
                          # aksi hâlde görsel yolu (~ açılır); "" = shell duvar kağıdı çizmez (hyprpaper vb.)
fit = "cover"             # cover | contain | fill (yerleşik her zaman çıkış boyutunda cover render edilir)
backdrop = "blur"         # contain letterbox zemini: blur | color
backdrop_blur = 48        # 4..=128 mantıksal px
backdrop_color = "surface"   # surface | surface_container | surface_bright
# [wallpaper.outputs."DP-1"] path = "..." / fit = "..."   # monitöre özel

[osd]
timeout_ms = 1200       # ses/parlaklık OSD'sinin görünme süresi, 500..=10000

[compositor]            # masaüstü (Hyprland) ortaklığı — docs/design/hyprland-sync.md
ensure_source = true    # açılışta Hyprland ayarında yoksa tek dahil etme satırı (hyprland.lua: pcall(require, "nuxut"); hyprland.conf: source), yedekli, bir kez
sync_look = true        # köşe (framework shape.radius_large), gölge, bulanıklık → pencereler
sync_motion = true      # framework [motion] → Hyprland animasyonları
sync_windows = true     # aralık + kenarlık + Hoş geldin penceresi kuralı (yüzer, ortada, sabit boyut)
sync_keybinds = true
sync_input = true       # [input] → Hyprland input:*; false ise [input]'tan hiç satır yok
base_layer = true       # temel katman (ADR-0006): pencere/çalışma alanı kısayolları, pencere ve katman kuralları, yerleşim/jest varsayılanları; hyprland.conf'un BAŞINDA source, kullanıcı ezer; false: temel dosyada yalnızca kabuk kısayolları

# [input] — sözleşme input-v1: HER anahtar isteğe bağlı, varsayılan YOK. Yazılmamış anahtar için
# üretilen dosyaya satır girmez → kullanıcının Hyprland değeri (ör. kb_layout = tr) kalır.
# Geçersiz değer (tür, ad deseni, aralık dışı — kırpılmaz) yalnızca o anahtarı düşürür + uyarı.
[input.keyboard]
layouts = [{ layout = "tr", variant = "" }, { layout = "us", variant = "intl" }]  # 1..=4; kb_layout+kb_variant birlikte
options = ["grp:alt_shift_toggle"]   # tam kb_options listesi, 0..=16; [] = açıkça temizle
repeat_rate = 25        # 1..=100 /sn
repeat_delay = 600      # 100..=2000 ms
numlock_by_default = false
[input.mouse]
sensitivity = 0.0       # -1..=1 (input:sensitivity; fare ve dokunmatik)
accel_profile = "adaptive"  # adaptive | flat
natural_scroll = false
[input.touchpad]
natural_scroll = false
tap_to_click = true     # input:touchpad:tap-to-click
disable_while_typing = true
scroll_factor = 1.0     # 0.1..=3
scroll_method = "2fg"   # 2fg | edge | on_button_down | no_scroll (input:scroll_method)

[windows]
gap_inner = 4           # 0..=64 px
gap_outer = 8           # 0..=128 px
border = 2              # 0..=16 px (renk: temanın primary/outline'ı)
blur = true
blur_size = 8           # 1..=32

[keybinds]              # eylem = "MOD+MOD+Tuş" | "" (atanmamış); verilmeyen varsayılanını alır
volume_up = "XF86AudioRaiseVolume"
panel_right = ""        # ör. "SUPER+N"; kullanıcının kendi bind'iyle çakışırsa üretilmez
shot_region = "Print"   # ekran görüntüsü; shot_output = "SHIFT+Print", shot_window = "ALT+Print"
magnifier_in = "SUPER+ALT+Up"  # büyüteç: magnifier_out = "SUPER+ALT+Down", magnifier_reset = "SUPER+ALT+Escape"
clipboard = "SUPER+SHIFT+V"   # başlatıcı pano kipi; emoji = "SUPER+period" (emoji kipi, docs/design/emoji.md)
launcher = "SUPER"      # tek değiştiriciye dokun (Super_L/Super_R bırakınca); temel katman tablosu: docs/design/hyprland-base.md §3.2
workspace = "SUPER"     # rakam ailesi: değiştirici + 1…0 (tuş kodu code:10…19, düzenden bağımsız); move_to_workspace = "SUPER+SHIFT"
close_window = "SUPER+Q"  # pencere: close_window_alt = "ALT+F4", fullscreen = "SUPER+F", focus_left = "SUPER+Left", …

[setup]
welcome = true          # setup.toml yoksa ilk açılışta nuxut-settings --welcome

[screenshot]            # docs/design/screenshot.md
dir = ""                # boş: $XDG_PICTURES_DIR/<yerelleştirilmiş "Ekran Görüntüleri">; mutlak ya da ~/…
save = true
copy = true             # wl-copy --type image/png
notify = true

[clipboard]             # docs/design/clipboard.md
enabled = true          # false: hiç okuma yok, bellekteki geçmiş bırakılır
max_items = 100         # 10..=500
images = true           # png/jpeg öğeler
persist = false         # $XDG_STATE_HOME/nuxut/clipboard/history.bin (0600, atomik)
primary = false         # birincil seçim de izlenir (data_control_watch_primary)
exclude_apps = ["org.keepassxc.KeePassXC", "KeePassXC", "Bitwarden", "1Password"]
x11_bridge = "auto"     # VM pano köprüsü (X11 ↔ Wayland): auto (VM konuk ajanı varken) | on | off; docs/design/vm-clipboard.md

[removable_media]       # docs/design/removable-media.md
enabled = true          # false: udisks'e hiç bağlanılmaz
notify = true           # takılınca "Aç / Çıkar" bildirimi (kendiliğinden bağlama yok)

[magnifier]             # docs/design/magnifier.md (Hyprland cursor:zoom_factor)
step = 1.25             # 1.05..=2: in çarpar, out böler
max = 8.0               # 1.5..=10 (Hyprland üst sınırı 10); set bunun üstünü reddeder
factor = 2.0            # kapalıyken açınca; oturumda son kullanılan tercih edilir
```
**Temel katman (ADR-0006, docs/design/hyprland-base.md).** `services::hypr_config::base` iki arka uca
`nuxut-base.conf` / `nuxut/base.lua` üretir: bütün kısayollar (kabuk + pencere/çalışma alanı; `nuxut:<eylem>`
açıklamalı), pencere kuralları (diyalog, dosya seçici, polkit, PiP, Ayarlar, hoş geldin, tam ekran video
idle-inhibit), Nuxut katman kuralları (animasyon, bulanıklık + `ignore_alpha`), yerleşim/davranış
varsayılanları ve 3 parmak jest. Satır kullanıcının ana dosyasının BAŞINA tek parça eklenir (açıklama
aynı satırda; `hypr.source` snapshot'ı; işaret `$XDG_STATE_HOME/nuxut/hypr-base-added`; yorumlanmışsa
`disabled_by_user`, silinmişse `removed_by_user`); `msg hypr status` `base`/`base_file`. Dosya değişince
`reload`. hyprlang taraması temel dosyaya girmez (`ScanInput::skip`). Masaüstü hareketi `generate::anim_plan`:
Material 3 vurgulu eğriler, bantlı kısa süreler, `popin 92%`/`slide`; `motion.enabled = false` → animasyon yok.

**Masaüstü ortaklığı (ADR-0003).** `app/hypr_sync.rs` → `nuxut-hypr-sync` thread'i
(`services::hypr_config::sync`). Ayar dili Hyprland'in sırasıyla bulunur (`-c`/`HYPRLAND_CONFIG`
→ `hyprland.lua` → `hyprland.conf`). Lua: `<hypr>/nuxut.lua` üretilir, `hyprland.lua` sonuna
`pcall(require, "nuxut")`; değer değişiminde `hyprctl eval`, bind değişiminde `reload` (Hyprland
`require`'lanan dosyayı kendisi de izler). hyprlang (geçiş dönemi): `nuxut.conf` + `source`,
`keyword` batch'i. Kullanıcının ayarında başka başlatıcı yoksa üretilen dosya oturum başında
`nuxut-shell.service`'i (systemd kullanıcı birimi, `dist/`) başlatır; systemd yoksa doğrudan.
İlk bağlantıda "Geri al" düğmeli bildirim (shell içi popup'lı bildirim; düğme = `msg hypr undo`); `msg hypr undo` yalnızca Nuxut'un satırını ve dosyasını
kaldırır. Kullanıcı ayarını Lua'ya taşıdıysa (`pending_switch`) yeni dil sonraki oturumda geçerli;
oturum başına bir kez "Şimdi uygula" düğmeli bildirim (`msg hypr apply-switch` → `reload full-reset`).
Bütün dış yazımlar framework `nuxut-apply` üzerinden (ADR-0001 §9.5; shell yapıştırıcısı
`services::apply`): ana dosyadaki satır `hypr.source` snapshot'ıyla (yedek `.bak` yok; geri al
da bir işlem, `msg snapshot restore` ile geri alınır), üretilen dosyalar türetilmiş (snapshot
yok, Nuxut başlığı taşımayan dosya ezilmez). Pencereler grubu Hoş geldin penceresi için tek
kural üretir (`generate::WELCOME_WINDOW`: app-id `nuxut-welcome`; boyut kuralda yok, istemci
xdg_toplevel min=max 720×640 verir): hyprlang `windowrule = match:class ^(nuxut-welcome)$, float on` /
`center on`, Lua `hl.window_rule({ match = { class = … }, float = true, center = true })`. Yalnızca dosya metnine girer (canlı `keyword`/`eval` her eşitlemede kuralı yinelerdi).
**Girdi (`[input]`, `services::hypr_config::input`, sözleşme `docs/design/input.md`).**
"İçe aktar, ezme": yalnızca `shell.toml`'da yazılı ve geçerli anahtarlar satır üretir (hyprlang
`input:kb_layout = …`, Lua `hl.config({ input = { …, touchpad = { … } } })`); "varsayılana dön" =
anahtarı silmek. Canlı uygulama: değerler `keyword`/`eval`; klavye düzeni/varyant/seçenek
değişimi ve kaldırılan anahtar `reload` (xkb bileşimi birlikte derlenir; kaldırılan değer
kullanıcınınkine döner — Lua'da yazılı anahtar kümesi `-- nuxut-input:` satırında izlenir).
Override tespiti (hyprlang): tarama `MANAGED_KEYS`'in (statik üst küme) yerini Hyprland'in okuma
sırasıyla bulur; override yalnızca bu turda **üretilen** anahtar için ve yalnızca Nuxut
dosyasının `source`'undan **sonra** okunan atamada raporlanır (öncekini Nuxut ezer; satır henüz
yoksa sona eklenir → ezen yok). Lua'da kaynak sırası taraması yok. `msg input status`
(`app/hypr_sync.rs`, ayrı thread): tek `[[BATCH]]j/getoption …` isteğiyle etkin değerler
(Hyprland `[[EMPTY]]` → `""`), `managed` (yazılı anahtarlar), `overrides` (`input.*` adıyla),
eklemeli `sync` alanı; Hyprland'e ulaşılamazsa `available:false` + `null`. `overrides` iki
kaynaktan: (1) son eşitlemenin dosya taraması (`file`/`line` dolu); (2) değer karşılaştırması —
yazılı her anahtarın `getoption` değeri ürettiğimizden farklıysa (`file`/`line` `null`; Lua'da tek
yol). (2) yalnızca uygulama **oturmuşken**: eşitleme işlerine sıra numarası verilir
(`hypr_seq_sent`), thread durumu işlediği numarayla döndürür (`SyncStatus::seq`, sırada bekleyen
daha küçük numaralı iş varsa onun altında kalır) → gönderilen son iş bitmiş, son tur canlı
uygulamayı hatasız yapmış (`SyncStatus::applied`) ve uygulanan istek bugünkü `[input]`'u
taşıyor olmalı; aksi hâlde uyuşmazlık "uygulama sürüyor" sayılır, override değil. Karşılaştırma
normalize: bool ↔ 0/1, ondalık ±1e-3, `kb_*` virgül listesi öğe öğe kırpılmış ve sondaki boşlar
atılmış, `[[EMPTY]]` = boş; beklenmeyen tür eşit sayılır. Dosya taramasının bulduğu anahtar
ikinci kez eklenmez (yeri bilinen tercih edilir). Nuxut dosyası birden çok kez source edilirse
sınır SON source'tur (Hyprland değerleri sonuncusunda uygular; `source_at` ilk satır kalır).
Bilinen sınırlar (inceleme, 2026-10-13): `device:<ad>:*` bölümleri `getoption`'da
görünmez, cihaz başına ezme yakalanmaz; kullanıcının betikle attığı `hyprctl keyword` değer
yolundan override olarak çıkar (doğru, ama yeri `null`).
Kapı `nuxut_apply::Env::guard`: Hyprland
imzası yoksa kapalı; test örneği (`NUXUT_SHELL_INSTANCE`) yalnızca gerçek ev dizini dışındaki
`XDG_CONFIG_HOME`/`XDG_STATE_HOME` altına yazar ve Hyprland'e komut göndermez (tek istisna: `NUXUT_TEST_HYPR_CONTROL=1`
ve `/run/user` dışında `XDG_RUNTIME_DIR` — yalnızca iç içe test Hyprland'ine ulaşılabilir).

**Birleşik görünüm (ADR-0004).** `services::look`: framework teması + `framework.toml [desktop]`
(`unified`, `icon_theme`, `cursor_theme`, `cursor_size`, `qt_backend`) ve `[colors] mode` →
ayar başına (`look.mode|icons|cursor|fonts|motion|qt_backend|unified`) parçalar: GTK4 `nuxut.css` (CSS
değişkenleri, iki kip `@media`), GTK3 `nuxut.css` (`@define-color`), `gtk.css` `@import` satırı,
`settings.ini`, dconf, `kdeglobals` + `Nuxut.colors`, yedek qt6ct/qt5ct, `index.theme`,
`kcminputrc`, `environment.d/99-nuxut-look.conf` (boş değerli değişken yazılmaz: systemd geçersiz sayar; ADR-0004 §17-11); ardından canlı eylemler (KDE sinyalleri,
`setcursor`, oturum ortamı). Rol kümesi `nuxut_palette::Palette::generate_with(…, theme.contrast)`.
Erişilebilirlik (framework t124 sözleşmesi, `../framework/docs/design/accessibility.md` §1–3;
ADR-0004 §18): yüksek karşıtlıkta dışa aktarılan 57 rol de WCAG AA eşiğine itilir (look.mode);
`Theme::reduced_motion()` → dconf `enable-animations` + iki `settings.ini` `gtk-enable-animations`
(`look.motion`); metin ölçeği her hedefe TAM BİR kez (`nuxut_palette::text_scale`): GTK yazı tipi
adı referans boyda (13 px), ölçek dconf `text-scaling-factor` + `settings.ini gtk-xft-dpi`
(look.fonts); kdeglobals QFont pt ölçekli; qt6ct yolunda yalnız `QT_FONT_DPI` (oturum ortamı,
ölçek 1 ya da kde yolunda yazılmaz ve canlı `UnsetEnvironment`); `QT_SCALE_FACTOR` asla. `[desktop]` şeması,
doğrulaması (`nuxut::theme::Desktop`, `QtBackend`) ve kip çözümü (`Theme::mode`; `[colors] mode`
varsayılan `auto`: palette.toml kipi → yüzey parlaklığı → koyu) framework'tedir; shell kopya
ayrıştırma yapmaz, GTK/Qt kipi Nuxut'un çizdiği kiptir. GTK CSS yönetilen başlığıyla framework'ten
(`nuxut_palette::look`). `nuxut-palette` `default-features = false` (shell ağacında jxl yok). Saf ve test edilir;
yazım tek kapıdan (`look::backend::LookBackend`): `look::writer::ApplyBackend` = `nuxut-apply`
(ayar başına tek `Change`/snapshot, kaynak `shell`) + canlı eylemler (`live_gate`: test örneği ve
ikincil örnek hiçbir sinyal/`setcursor`/oturum ortamı göndermez; test örneğinde dconf parçaları
düşülür, dosyalar yalnızca test köklerine). Çalışma anı `app/look.rs` (`nuxut-look` thread'i):
açılışta ve `theme_changed`'de (framework `[desktop]` değişimini de görsel sayar) görünüm girdisi
(kip, renkler, karşıtlık, azaltılmış hareket, tipografi, masaüstü seçimleri) değiştiyse tur; thread 120 ms birleştirir, tur sürerken gelen yeni istek turu ayarlar arasında keser ve canlı eylemleri son tamamlanan tura bırakır ("son seçim kazanır", docs/design/look-external.md §5); aynı
değerler `NoOp` (snapshot/sinyal yok); yazımlar türetilmiş (`apply_derived`: araya kimse girmemişse
son işleme birleşir, yeni snapshot yok; `gtk.css` `write_creating`). `unified = false` ve Nuxut'un yazdıkları diskteyse (bir
`look.*` ayarının en yeni işlemi uygulanmış ve geri yükleme değil) sabitlenmiş işlemler yeniden
eskiye zorla geri yüklenir (ADR-0004 §8-3; ayarlar arası sıra serbest, paylaşılan Nuxut dosyası boşalınca silinir — nuxut-apply `file_inherited`) + KDE sinyalleri, systemd `UnsetEnvironment`,
geri yüklemelerin döndürdüğü `settings.ini` imleciyle (`restore_with_values`) `setcursor`. İmleç ortamı `hypr_config` üreticisine girdi
(`SyncRequest::cursor_env` → `env =` / `hl.env`; canlı uygulanmaz); ilk görünüm turu bitene kadar
Hyprland eşitlemesi bekler. İkincil örnekte thread başlamaz (`writer: disabled`). `msg look
status|plan` salt okunur JSON (`writer`, `last_apply`; sürüm 4: `themes_rev`; sürüm 5: `last_apply.external`,
`last_apply.paused`; sürüm 6: `last_apply.taken_over`). **Yeniden yazım bekçisi** (`look::guard`, docs/design/look-external.md; 2026-10-16
hatası): plan önceki uygulanan planla aynıysa tur atlanır (tetik ne olursa olsun yazım/sinyal/log yok;
dışarıdan geri alınan değer girdi değişene ya da `look apply`'a kadar dayatılmaz); 60 sn'de 10'dan fazla
gerçek yazım alan ayar 5 dk durdurulur (tek uyarı); hata/dış hedef uyarıları yalnızca değişince. dconf
değerleri `dconf read` biçiminde üretilir (`gtk::gvariant_double`: `1.0`, `1.1000000000000001`), yoksa
`nuxut-apply` metin karşılaştırması her turu değişiklik sayar. Başka aracın yönettiği dosya
(`ExternallyManaged`) varsayılan olarak **devralınır** (kullanıcı ilkesi 2026-10-16; `nuxut-apply`
`Change::take_over`: symlink — hedefi root'un tema dosyası olsa da —, salt okunur, sabit bağ; özgün
biçim aynı snapshot'ta, kapatma/"önceki hale dön" geri koyar). Kullanıcının GTK CSS'i başka bir
temaya bağsa (`gtk-4.0/{gtk,gtk-dark}.css`, `gtk-3.0/gtk.css`; `SystemProbe::foreign_link`) içerik
Nuxut'un yönetilen dosyasıyla değiştirilir (tema CSS'i Nuxut renklerini ezerdi). Yürürlükteki
devralmalar `last_apply.taken_over` (`~/…` yol; uygulanmış `look.*` snapshot'larının `file_form`
parçaları, yol bugün bağ değilse). Devralınamayan (nix/guix deposu, başka sahibin düz dosyası,
yazılamayan dizin) ayarı düşürmez: o dosyanın parçaları çıkarılıp kalan tek işlemde yazılır, dosya
`last_apply.external` (`~/…` yol + neden) ile bildirilir. Nuxut GTK teması, kitty (renkler +
imleç izi, `kitty_reload` = SIGUSR1) ve Nuxut Qt stili (ADR-0009; `qt-style.conf`,
`QT_PLUGIN_PATH`) için docs/design/look-gtk.md. Renk teması seçiliyken (`Theme::color_theme`) dış
palet `Scheme::from_color_theme` ile temanın tanımından kurulur (tohumlama yok). Kurulu tema izleyicisi
`look::watch` (`nuxut-theme-watch` thread'i, her örnekte): `~/.icons`, `$XDG_DATA_HOME/icons`,
`$XDG_DATA_DIRS/*/icons` ve aynı sıradaki `themes/` dizinleri + her tema dizininin yalnızca üst düzeyi
(`index.theme`, `cursors/`, hyprcursor manifesti, `gtk-3.0/`, `gtk-4.0/`) inotify ile, özyinelemesiz;
yok olan dizin en yakın üst dizinden izlenir (yalnızca yoldaki ad uyandırır). 500 ms sessizlik / en
geç 2 sn birleştirme; sonra izleme yeniden kurulur, parmak izi (tema adları + işaretlerin boy/zamanı)
farklıysa `themes_rev` artar ve görünüm turu aynı girdiyle de istenir (yeni kurulan istenen tema
yedeğin yerine geçer). Kurulu listeler her `status`'ta taze taranır (önbellek yok). inotify açılamaz
ya da limit (`ENOSPC`) dolarsa uyarı + izleyici kapanır (`themes_rev` 0'da kalır). `nuxut-shell look apply` (`look::oneshot`):
aynı turu süreç içinde bir kez çalıştırır (çalışan shell gerekmez, ona IPC yok; eşzamanlılık
`nuxut-apply` kilidiyle, aynı girdi `NoOp`); `NUXUT_SHELL_INSTANCE` varsa test örneği kuralları.
İmleç ortamını Hyprland yapılandırmasına yazmaz (çıktıda bilgi). JSON; çıkış 0 tam, 1 kısmi, 2 okunamadı. `nuxut-shell
look revert`: `framework.toml`'a bakmadan kapatma yolunu (§8-3, `writer::deactivate`) bir kez çalıştırır —
görünüm Nuxut öncesine; Nuxut'un yazdığı bir şey yoksa `report: null`. install.sh test dönemi taze
kurulumu kullanır (çalışan shell önce kapatılır, yoksa bir sonraki turda yeniden uygular).
(`dnd`/`enabled`/`apps` adları `notifications`'ın modelinde uygulanıyor; o güne kadar model
`do_not_disturb` okur.)

**İlk açılış / Hoş geldin (ADR-0001 §10).** Durum `$XDG_STATE_HOME/nuxut/setup.toml`'da
(yoksa `~/.local/state`): `state = "done"|"skipped"`, `version` (sihirbaz sürümü 1..=65535),
`at` (RFC 3339). **Tek yazan shell'dir** (`services::setup`, `app/setup.rs`): Ayarlar sihirbazı
bitirince/atlayınca `msg setup done|skip <sürüm>` çağırır; daha küçük sürüm büyüğü düşürmez
(no-op, `ok`). Okuma 4 KiB sınırlı; bozuk dosya ilk açılış sayılmaz (uyarı loglanır,
pencere her açılışta yeniden açılmaz; `setup done/skip` onarır). Yazım atomik, dizin 0700
oluşturulur. Yazım kapısı shell.toml'unkiyle aynı: kilidi almış varsayılan örnek; test örneği
yalnızca `~/.local/state` olmayan mutlak bir `XDG_STATE_HOME`'a. Açılışta dosya **yoksa**,
`[setup] welcome = true`, varsayılan örnek ve Hyprland oturumuysa `nuxut-settings --welcome`
ayrık başlatılır (`setsid -f`, beklenmez; önce shell'in dizini, sonra PATH; bulunamazsa uyarı).
Oturum başına bir kez: `$XDG_RUNTIME_DIR/nuxut/welcome-<imza>` işareti, shell çökerek yeniden
başlarsa pencere ikinci kez açılmaz. `msg setup status` → tek satır JSON:
`{"first_run","setup_version","setup_state","hypr":{"session","source","launch"},"migration":{"pending_notice"}}`.

**Güncelleme bitti bildirimi (2026-10-13).** install.sh sinyali yok: kabuk açılışta
(`start`, Hoş geldin'den sonra) `CARGO_PKG_VERSION`'ı `$XDG_STATE_HOME/nuxut/shell-state.toml`
`last_version` ile SemVer önceliğiyle karşılaştırır (`services::shell_state`; 4 KiB, atomik
`nuxut-apply`, dizin 0700). Artışta tek popup'lı iç bildirim ("Nuxut güncellendi", gövde yeni
sürüm) + kartta maskot `Celebrate` — framework `NotificationCard` maskot yuvası (fw 1c9174a; 64 px,
metnin solunda, karosuz; shell'deki geçici 56 px görsel yuvası yolu kalktı, 2026-10-14);
`[appearance] mascot = false` → yuvayı kart kapatır, kart kısalır (tema değişiminde kart korunur,
measure + set_rect yeniden dizer). Dosya yoksa
(ilk kurulum) yalnızca yazılır; aynı/düşük sürüm bildirimsiz (düşükte yazılır); bozuk dosya
uyarı + onarım. Bildirim yalnızca yeni değer yazıldıysa çıkar (yazamayan örnek hiç göstermez).
Maskot alanı (`InternalNotice::with_mascot`) yalnız iç bildirimlerde; D-Bus istemcisi
ayarlayamaz. Hoş geldin ekrandaysa (`offer_welcome` `true`) bildirim BASTIRILIR, `last_version`
yine yazılır — ikisi asla birlikte çıkmaz (2026-10-13). Sürüm yalnız sürüm yayınında
artar (2026-10-13): test döneminde `0.1.0` kalır, bildirim görünmez (kabul);
install.sh'ye bir şey eklenmez; `shell-state.toml` taze kurulum listesine girmez. Ayrıntı:
`docs/design/mascot-placement.md`.

Sürüm göçü (ADR-0002, `src/config_migrate.rs`): dosyanın üstünde `config_version` (yoksa 1;
güncel 2). Eski sürümlü dosya açılışta / `config reload`'da bir kez, yorumlar ve sıra korunarak
taşınır ve atomik yazılır; adımların değiştirmediği sürümsüz dosyaya yalnızca bir kez `config_version`
damgası yazılır (dosyanın kendiliğinden yazıldığı tek durumlar; yalnızca tek-örnek kilidini almış
varsayılan adlı örnek yazar, test örneği bellekte çalışır; salt okunur dosyaya yazılmaz). Güncel ya
da daha yeni sürümlü dosya yazılmaz. v2 adımları: `bar.left`/`bar.right`
→ `bar.start`/`bar.end`, `[workspaces]`/`[clock]` → `[bar.widget.*]`, `frame.inner_radius`
silinir, `[bar.widget.workspaces]` `show_empty` kalkar ve `count` sayfa boyu olur (görünmeyen
`count` silinir, >30 → 30; bunlar bildirilir, `msg config undo-migration` eski değeri yeni
anlamıyla geri koyar). Güncel sürümde kalan eski, geçersiz, tekrar eden ya da varsayılana eşit
satırları ve bozuk TOML'u **kendini onarma** düzeltir (`src/config_heal/`, `crates/nuxut-config-heal`;
açılışta yedekle yazar, dosya değişince yalnızca bellekte; `msg config status` durum, `msg config undo`
geri al (kabuk yeniden başlasa da), `msg config heal [--dry-run|--undo|
--fix-typos|--ignore-typos]`). Şema anlık görüntüsü `docs/config-schema/shell-v<sürüm>.txt`: anahtar
kaldıran ya da türünü değiştiren değişikliği test yakalar. Ayrıntı: `docs/design/config-migrations.md`.

Doğrulama (`src/config.rs`, `src/widgets/registry.rs`): değerler sınırlanır ve uyarı verilir —
kalınlıklar 0..=64, `bar.size` 16..=128, `bar.padding`/`spacing` 0..=64,
panel `size` 160..=900, `workspaces.count` 1..=30, saat biçimi geçerli strftime (ay/gün adları ve `%x`/`%X`/`%c` zaman locale'inden: `framework.toml [locale] formats`, `auto` ise `LC_ALL` > `LC_TIME` > `LANG`, yoksa `en_US`; 12/24 saat `[locale] hour_cycle`; takvim popout'unun uzun tarihi, büyük saati ve etkinlik saatleri de aynı locale; boş kalıp = otomatik); NaN/sonsuz →
varsayılan; bilinmeyen/tekrarlanan/bu yere konamayan widget atılır; bilinmeyen widget tablosu ve
anahtarı uyarı.

Kısmi tablo yalnızca yazılan anahtarları değiştirir: `[panels.right] size = 400` sağ panelin
`enabled`/`widgets` varsayılanını korur (kenar başına varsayılanlar `PanelsConfig::default`).

### Ayarlar şeması (`nuxut-shell schema`)

Ayarlar uygulaması formlarını `nuxut-shell schema` çıktısından kurar (TOML, çalışan shell
gerekmez, `src/schema.rs`): ilk satır `version = 1` (uyumsuz değişiklikte artar); `[[widget]]`
(`name`, `title`, `icon` = framework `IconName` adı, `bar`, `panel`, `[[widget.setting]]`) —
sıra `WidgetKind::ALL`, yani "widget ekle" menüsünün sırası; `[[setting]]` genel anahtarlar,
noktalı yol (`bar.size`, `notifications.max_popups`). Her ayar: `key`, `title`, `kind`
(`bool`/`int`/`float`/`text`/`choice`/`custom`), `default`, sayılarda `min`/`max`/`step`, `unit`,
seçimde `options`. `custom`: yapılı değer (liste/tablo listesi, ör. `input.keyboard.layouts`);
`default` satır içi TOML dizisi, Ayarlar kendi bloğuyla yazar. `[input]` anahtarlarının
`default`'u yalnızca Hyprland varsayılanının göstergesidir (config'te serde varsayılanı yok).
`unit` dile bağımsız kapalı bir sembol kümesidir (`schema::UNITS`: `px`, `%`, `ms`, `s`, `/s` — saniyede);
çevirisini Ayarlar kendi kataloğundan yapar, şema çıktısı locale'den bağımsız kalır (karar:
2026-10-13 — Türkçe "sn" yerine; alternatif shell'de `t!()` ile çevrilen birim, şemayı
dile bağlayacağı için seçilmedi).

Kaynak tektir: varsayılanlar config yapılarının `Default`'undan, sınırlar doğrulamanın kullandığı
sabitlerden (`MAX_BAR_SIZE`, `MAX_POPUP_COUNT` …); şemaya elle sayı yazılmaz. Her config bölümü
kendi anahtarlarını kendi `schema` fonksiyonunda kaydeder (`FrameConfig::schema`,
`NotificationsConfig::schema`, `WidgetKind::settings_schema` …). **Kural: yeni config anahtarı
eklenince o bölümün `schema`'sına da eklenir** (yokluğu anlamlı `Option` ve serbest tablolar —
`critical_timeout_ms`, `notifications.apps`, `wallpaper.outputs` — hariç). Testler: şemadaki her
anahtar config'te var ve varsayılanıyla yazılan config uyarısız/varsayılana eşit; aralık uçları
uyarısız, dışı uyarı ya da hata; izin verilmeyen seçenek uyarı ya da hata; ikonlar geçerli
`IconName`.

## Widget kaydı

`src/widgets/registry.rs` tek kayıt yeridir: `WidgetKind` (ad, yerleşebileceği yer: bar/panel),
ayar yapısı (`[bar.widget.<ad>]`), doğrulama. Çalışma zamanı: `BarWidget` trait'i
(`sync`/`set_time`/`measure`/`draw`/`hover`/`click`) ve `widgets::create` fabrikası; her widget
kendi dosyasında (`src/widgets/*.rs`). Yeni widget = kayıtta varyant + ayar + fabrikada bir kol +
dosyası. Bar yerleşimini (eksen, bölümler, kayma) `Screen` yapar; widget yalnızca kendi
dikdörtgenini bilir.

Widget sözleşmesinin geri kalanı (`src/widgets/mod.rs`):
- **Overlay kancaları** (`BarWidget`): bar dışına taşan içerik (tepsi menüsü, popover) şeklini
  shell'in blob alanına verir (`overlay_shapes` — bar'dan blob olarak büyür, fileto ile birleşir),
  içeriğini en üstte çizer (`draw_overlay`), input bölgesine dikdörtgen ekler (`overlay_input`),
  işaretçi ve klavyeyi bar'dan önce alır (`overlay_pointer`, `key`, `wants_keyboard`),
  Escape/odak kaybında kapanır (`close_overlay`). Açılış/kapanışta `Action::RefreshInput`.
  Sınırı (`WidgetCx::bounds`) iç alan eksi görünen sabit paneller (`panels::free_area`); bar
  menüsü açılırken geçici yaprak kapanır — "tek geçici yüzey", `docs/design/panels.md` §5.
- **`PanelWidget`**: kenar çekmecesi içeriği; dikey yığın, `height() == None` olan esnek widget
  kalan alanı alır (bildirim merkezi). İçerik yaprağın görünen dikdörtgenine yerleşir, opaklıkla
  belirir. Panelde yalnızca kayıtta `placement().panel` olan türler: `session` (oturum kartı),
  `notifications` (merkez).
- **İmleç** (framework DESIGN "İmleç"): `cursor(rect, p)` / `overlay_cursor(p)`; `Screen` en
  üstteki isabetten seçer (overlay → açık yaprak → bar), App `cx.set_cursor` ile uygular.
  Tıklanabilir = `Pointer`, zemin = `Default`.
- **Renkler** yalnızca temadan; tema değişince renkler `AnimatedTheme` ile `motion.effects`
  hızında akar. `tests/no_hardcoded_colors.rs` bekçisi: `src/` ve `examples/` içinde koda gömülü
  renk yasak (`// renk-matematiği:` ve dış veri için `// renk-verisi:` işaretli satırlar hariç).
- **Karışımlar ve yüksek karşıtlık** (framework t124, `../framework/docs/design/accessibility.md`
  §1 "Bileşen karışımları"; uygulama 2026-10-13): shell bir rengi
  alfayla soldurduğunda ya da bir katman bindirdiğinde bunu elle yazmaz, `nuxut::ui::style`
  yardımcısıyla yapar — standart kipte piksel aynı, yüksek karşıtlıkta eşik (metin ≥ 4.5, işaret
  ≥ 3) gerçek birleşik zemine karşı sağlanır. Bar öğelerinin hover/basılı katmanı yalnız
  `widgets::draw_hover` ile (`Interaction::draw_state_layer_under`; zemin: açık popout hapı
  `surface_bright`, workspace tonal diski ya da aktif hap `primary`; ölçüt üstteki içerik
  renkleri). Workspace boş yuvası `faded_text`/`faded_mark`, tonal disk `state_layer`; gündem
  noktası `faded_mark`; oturum kartında onay beklerken soluyan düğmelerin grup opaklığı
  `disabled_content(on_surface, [tonal_container]).a` (devre dışı kararı: yüksekte ≥ 3).
  Kapsam dışı (bilerek): animasyon opaklıkları (belirme/kayma `t`, kart `alpha`), tam ekran
  karartmalar (ekran görüntüsü, boşta), kilit ekranı marka işareti (%60; logo, WCAG 1.4.11 muaf).
  Hyprland pasif pencere kenarlığı artık kapsamda: ADR-0004 §18.6 (yüksekte tema `surface`a ≥ 3).
- **Dil değişimi** (ADR-0010; `App::locale_changed`, süreç geneli katalog zaten yeni; yalnız `[locale] hour_cycle` değişimi de buraya gelir, saat timer biçimi yeniden hesaplanır; saat dilimi değişimi aynı `relocalize_screens` yolunu kullanır):
  `Screen::relocalize` = aynı config ile zorlanmış `apply_config`. Bar ve çekmece widget'ları
  türlerinden yeniden yaratılır (`widgets::recreate_bar`/`recreate_panel`), modelin ve saatin son
  durumundan eşlenir — servis olaylarından biçimlenen metin de son durumdan yeniden biçimlenir;
  bu yüzden widget metnini `t!()` ile kurucuda, `sync`'te ya da `set_time`'da kurar ve olaydan
  gelen durum yalnızca widget'ta değil modelde (`ShellModel`) durur. Durumlu görünümler kendini yeniler
  (`relocalize`): başlatıcı (sorgu, odak, seçim, eylem sayfası korunur), takvim (seçili gün/ay),
  sistem izleyici (son ölçüm), bildirim kartları (yeniden kurulur ve ölçülür; dokular kalır).
  Yaprak durumu, sabitler ve odak korunur; açık çekmecedeki yeni widget'ların açılış kancaları
  çalışır (eylemleri App uygular), içeriğe göre boyutlanan açık popout'lar yeniden hedeflenir,
  ipucu kapanır, tam hasar. Widget'ın kendi geçici durumu (açık tepsi menüsü, onay bekleyen
  düğme, kaydırma) sıfırlanır. Servisler yeniden bağlanmaz.

## Yapraklar: popout ve kenar çekmeceleri

`src/sheet.rs` `Sheet`: kenardan büyüyen blob şekli (tohum → hedef, yay, hız deform'u, kapanışta
şeride gömülme, oturunca alandan çıkma, içerik opaklığı eşikleri, hasar). Saat popout'u bar
kenarından saatin hizasında büyür; kenar çekmeceleri (`[panels.<kenar>]`) çerçeve kenarından iç
alanın tam boyunda. Aynı anda tek **geçici** yaprak açıktır; kenar panelleri ayrıca **sabitlenebilir**
(köşedeki pin düğmesi ya da `msg panel pin`): sabit panel dışarı tıklama, odak kaybı, Escape ve
başka yaprağın açılmasıyla kapanmaz, iç alanı sırayla oyar (geçici yaprak kalan alanda), yalnızca
sabitken klavye `OnDemand`. Durum `src/panels.rs` `PanelState` (saf), kurallar
`docs/design/panels.md`. Açma: bar widget'ı (session, notifications,
panel-toggle, clock) ya da komut; kapatma: Escape, dışarı tıklama (odak kaybı), aynı düğme (sabit
panelde yalnızca aynı düğme / komut).

**Saat/takvim popout'u** (`src/calendar_view.rs`): büyük saat, uzun Türkçe tarih, ay ızgarası
(`CalendarGrid`; hafta başı yerel ayardan, gün başına ≤3 takvim renkli nokta) ve seçili günün
etkinlikleri (tüm gün önce, ≤5 satır + "+N daha"; ok tuşlarıyla taşınan seçimi izler). Daemon
yoksa ızgara çalışır, liste "Takvim servisi çalışmıyor" der. Genişlik 360, yükseklik içerikten.
Aynı `CalendarView` kenar panelinde **Takvim kartı** (`calendar` widget'ı, yalnızca panel;
`CalendarMode::Card`: başlıksız, ≤3 satır, içerik ≤320 px ortalı) olarak da kullanılır.
Sorgu: servis tek aralık alır; görünür tüm takvimlerin (açık popout'lar, açık/sabit panellerdeki
kartlar; `Screen::calendar_ranges`) 6 haftalık aralıkları `calendar_view::query_range` ile
kapsayan tek aralığa birleştirilir (400 günü aşarsa son ay değiştirenin aralığı) ve yaprak
açılıp kapanınca, ay değişince (`Action::CalendarRange`), config/dil değişince yeniden sorgulanır
(`app/calendar.rs` `CalendarQuery`). Karar ve sınırlar: `docs/design/calendar-card.md`.

**Sistem izleyicisi popout'u** (`src/sysmon_view.rs`, `PanelId::Sysmon`): bar'daki `sysmon`
widget'ının hizasından büyür (600 px, yükseklik içerikten; içerik boyu değişince yaprak yeni
hedefe akar). İki sütun: solda işlemci (çubuk, çekirdek başına mini çubuklar — en çok 32, yük,
frekans), bellek + takas, GPU'lar (uykudaysa "uykuda"); sağda sıcaklıklar (≤ 6), ağ (indirme/
yükleme), disk (bağlama noktası başına çubuk ≤ 4, okuma/yazma); altta tam genişlikte en çok CPU
kullanan süreçler (ad, %, bellek). Olmayan ölçümün bölümü yoktur. Çubuklar framework `Progress`,
kullanım ≥ %90'da `Threshold` rengine geçer (çekirdek ve bağlama noktası çubukları dahil). İşlemci,
bellek ve ilk GPU başlığının altında servis geçmişinden framework `Sparkline` (0..100 sabit eksen,
aynı eşik): ardışık `seq` gelirse yalnızca yeni örnek itilir (kayar), aksi hâlde geçmiş
animasyonsuz yüklenir. GPU ikonu `IconName::Gpu`.
Etkileşimsiz; açıkken servis ayrıntı kipindedir. Açma: widget ya da `msg panel open sysmon`.

**Başlatıcı** (`src/launcher_view.rs`, `PanelId::Launcher`): bar kenarının ortasından büyüyen
yaprak (`[launcher].width` × arama alanı + `rows` satır). `SearchField` + sanal `ResultList`
(eşleşen harfler vurgulu; vurgu yalnız ilk 100 sonuç için, boş sorguda tüm uygulamalar).
Klavye: yazmak arar; ↑ ↓ PgUp PgDn Home End seçer; Enter başlatır; → (imleç sonda) masaüstü
eylemleri alt sayfası (`PageStack` morfu, "‹ Ad" başlığı), ← / Backspace / Esc geri; Esc önce
metni temizler, boşsa kapatır. Hesap satırı en üstte; Enter sonucu alana yazar (pano gelince
kopyalayacak). İkonlar seçimin çevresindeki ≤ 24 satır için `nuxut-image` Loader'ıyla arka
planda (tema araması + çözme), ≤ 384 ikon önbellekte. Açma: bar'daki `launcher` düğmesi, `logo` düğmesi ya
da `nuxut-shell msg launcher toggle` (bind ile açılan yaprak kapanana kadar `Exclusive` klavye).
Kipler (`LauncherMode`; aynı yaprak ve satırlar): uygulamalar (varsayılan), pano
(`msg launcher clipboard`) ve emoji (`msg launcher emoji`; `docs/design/emoji.md`). Emoji
ızgarası: framework `GlyphGrid` (`src/emoji_view.rs`; boş sorguda "Sık kullanılanlar" + Unicode grup
bölümleri, sorguda düz küme), altında seçili emojinin büyük glifi ve iki dildeki adı; kurulu emoji
fontunun tek glife indiremediği emoji gizlenir (tofu yok);
Enter/tık → `Action::PickEmoji`: kabuğun pano sahipliğiyle panoya (`cx.set_clipboard`), kullanım
kaydı servis thread'ine, yaprak kapanır. Odaktaki pencereye yazma (enjeksiyon) yok (v2). Kısayol
`SUPER+.` (eylem `emoji`; karar 2026-10-13, kullanıcı onayına açık).

**Marka işareti** (framework `nuxut-ui::BrandMark`, Kara Sağan; DESIGN "Marka işareti"): renkler
yalnızca tema rolünden, ≥ 24 px mantıksal (kuyruk 16 px'te incelir). Üç yerde:
- `logo` bar widget'ı (`src/widgets/logo.rs`): başlatıcıyı açar/kapatır (`launcher` düğmesiyle
  aynı yaprak ve eylem); varsayılan `bar.start`'ın başında — yalnızca serde varsayılanı, göç adımı
  yok: `bar.start`'ı açıkça yazmış kullanıcıya eklenmez. Kare ikon düğmeleriyle aynı dil (dairesel
  state layer, açıkken `surface_bright` + `primary`, boşta `on_surface` tek renk); kenar =
  öğe boyunun 4/3'ü, en az 24 (varsayılan 34 px bar'da 24, her yanda 5 px), bar kalınlığından her
  yanda `space(1)` pay. Statik: hasar yalnızca hover/vurgu/tema değişiminde, widget dikdörtgeni;
  ilk doku yüklemesi kare bütçesine takılırsa tek ek kare. Uç eylemleri (`[bar.edge_actions.start]`)
  artık en baştaki bu widget'a uygulanır.
- Kilit ekranında saatin üstünde, ortalı 32 px, `on_surface_variant` %60 opaklık
  (`lock/src/view.rs` `mark_rect`).
- Oturum kartı başlığının sağ ucunda, avatarla aynı hizada 24 px `on_surface_variant`; ad ve
  ayrıntı satırı tek satırda işarete değmeden kesilir; köşeye iç boşluk kadar uzak (sabitleme
  düğmesi köşe boşluğunda kalır).

**Oturum kartı** (`src/widgets/session_card.rs`): avatar, ad, kullanıcı · açık kalma süresi;
kilitle / oturumu kapat / askıya al / yeniden başlat / kapat. **Her eylem** (kilitle ve askıya al
dahil; kullanıcı kararı) **yerinde onay** ister: ilk basışta düğme hapa açılır ("<Eylem> · tekrar
bas", ikon çevresinde `theme.timing.confirm_window` (4 sn) geri sayım halkası; hareket kapalıyken
saniyelik adımlar), ikinci basış uygular (kuruluştan sonraki `confirm_guard` 400 ms içindeki basış
yok sayılır); süre dolunca, Escape'te, hap dışına basışta ya da panel kapanınca vazgeçilir (soru
metni ve Vazgeç düğmesi yok). Her düğme framework'ün `nuxut::ui::ConfirmButton`'ıdır (durum
makinesi, içerik takası, halka, kare/uyanma yönetimi onda); kart yalnızca yerleşimi (hap kendi
yerinde, satıra sığdırılmış), beklerken diğer düğmelerin soluk ve basışsız olmasını ve olay
yönlendirmesini yapar. Hap rengi (`with_armed_variant`): geri alınamayanlar (oturumu kapat, yeniden
başlat, kapat; `SessionAction::is_destructive`) uyarı (`ButtonVariant::Destructive`), kilitle ve
askıya al vurgu (`Filled`, primary). logind izin vermiyorsa düğme devre dışı (`set_enabled`
bekleyen onayı düşürür); son hata kartta kısa ve anlaşılır yazılır ("Kilitle başarısız: İzin
verilmedi"): servis D-Bus hata adını `FailureReason`a eşler (izin yok, yetki gerekiyor, işlem sürüyor,
desteklenmiyor, oturum yok, ulaşılamadı, zaman aşımı, diğer), ham ayrıntı yalnızca loga yazılır;
metin kart genişliğinde sarılır, en çok iki satırda `…` ile kesilir.

**Oturumu kapat** (`services/logout.rs`, `docs/design/logout.md`; 2026-10-16 "siyah ekran" hatası):
önce compositor temiz çıkar — uwsm altındaysa `uwsm stop`, değilse Hyprland `dispatch exit` (Lua
kipinde `dispatch hl.dsp.exit()`); hiçbir yol işlemezse ya da kabuk 10 sn sonra hâlâ yaşıyorsa
logind `Session.Terminate()` yedeği. Test örneği oturumu kapatmaz.

**Bildirimler.** Popup yığını (`src/widgets/notification_popups.rs`) Screen'in blob alanında,
bar kenarındaki iç alanın end köşesinden (yatay bar'da sağ, dikey bar'da alt) büyüyen tek bir
yapraktır (genişlik 380); yaprakların üstünde, overlay menülerin altında çizilir, işaretçide
menüden sonra ilk sırada. Kenar çekmecesi açıkken çöker (merkez orada). Bildirim merkezi sağ
çekmecede oturum kartının altında; çekmece açılınca `PanelWidget::panel_opened` →
`MarkAllRead`. Gövde bağlantısı → `NotifyHandle::open_link` (portal OpenURI). Görsel
çözünürlüğü: çıkış ölçeklerinin en büyüğü `NotifyHandle::set_output_scale`'e, tepsi ikonları
`TrayHandle::set_icon_size(icon_size × ölçek)`'e verilir (`Shell::sync_notify_scale`,
`Shell::sync_tray_icon_size`; yalnızca değişince).

**Tepsi.** Menü `WidgetCx::bounds`'a (çerçevenin hedef iç dikdörtgeni) sığacak şekilde bar'dan
büyür; dbusmenu'süz öğede sağ tık SNI `ContextMenu(x, y)` (uygulama kendi menüsünü gösterir).
Her menü açılışı süreç genelinde artan bir istek kimliği alır (`next_menu_request`,
`OpenMenu { request }`); servis o açılışın ağaçlarını `MenuLayout { request }` ile yayınlar,
önceki açılışın geç okumasını atar ve aynı anda tek menüyü izler. Model aynı ağaç yeni
kimlikle gelince de değişir (UI yalnızca model değişince `sync` eder; aksi halde aynı menüyü
ikinci kez açmak sessizce başarısız oluyordu). Widget yalnızca beklediği kimliğin ağacını
gösterir; açık menünün ikonuna tıklamak yalnızca kapatır, bekleyen açılış yeni bir basmayla
iptal olur.

## Gece ışığı

Tasarım ve gerekçe: `docs/design/night-light.md` (t113). Akşam renk sıcaklığını düşürür; mekanizma
Hyprland `hyprland-ctm-control-v1` (çıkış başına 3×3 CTM), kabuğun içinden — `hyprsunset` süreci yok.

- **Servis** `src/services/night_light/` (UI'dan bağımsız): `model.rs` saf çekirdek (Tanner Helland
  Kelvin → RGB, 6500 K'ya normalize = birim; zamanlama `always`/`manual`/`sun` → yerel takvimde
  ±2 günlük UTC gece aralıkları; sınırın ortalandığı `transition` geçişi mired uzayında; DST: var
  olmayan yerel saat boşluğun sonrasına, iki kez yaşanan erken olana; kutup günü/gecesi `sunrise`
  `None` + güneş sapmasıyla), `location.rs` (TZ / `/etc/localtime` → `zone1970.tab` koordinatı),
  `config.rs` (`[night_light]`), `mod.rs` durum makinesi `NightLight`.
- **Arka uç** `ColorTransform` arayüzü (`supported`, `set(output, [f32; 9])`, `commit`, `release`);
  her çağrıda verilir. `app/night_light.rs`: framework renk dönüşümü (`set_color_transforms`,
  `color_transform_release`, `App::color_transform_blocked`; fw e6f9d4f); çağrılar sıraya girer ve
  her makine çağrısından sonra compositor'a gider. Compositor sunmuyorsa **desteklenmiyor** → kart
  gizli, `msg night-light` hata. Test örneğinde (`NUXUT_SHELL_INSTANCE` dolu)
  `NUXUT_NIGHT_LIGHT_FAKE=1` çağrıları yalnızca loglayan sahte arka uç (gerçek CTM'e dokunmaz).
- **Başka istemci** (ör. kullanıcının hyprsunset'i) sahipse `blocked`: durum gösterilir, yeniden
  denenmez; elle açma bir kez dener. Kabuk ölünce Hyprland CTM'i birime döndürür.
- **Uyanma:** tek timer, yalnızca bir sonraki değişim anına (geçişte ≤ 30 sn / ~50 K adım, dışında
  bir sonraki geçişin başı; `always` ve kapalıyken hiç). Uykudan uyanma (logind
  `PrepareForSleep(false)`, `app/idle.rs`) ve saat sıçraması (var olan saat timer'ının atımında
  duvar/monoton saat farkı > 2 sn) hemen yeniden hesaplar.
- **Kapalıyken CTM'e dokunulmaz;** kapanınca yalnızca `release` (birim set değil). Arka uca iki uygulama arası
  ≥ 33 ms (kaydırıcı ≤ 30 Hz; arada gelen son değer timer'da uygulanır). Yeni çıkış son matrisi alır.
- **Elle geçersiz kılma** (kart anahtarı, `msg night-light on|off|toggle`): zamanlamanın aynı
  duruma yerleştiği bir sonraki geçiş sonuna kadar ("yarına kadar kapalı"); zamanlama zaten o
  durumdaysa kılma yok; `always` ya da `enabled = false` iken oturum boyunca. Kaydırıcı/`temperature`
  oturum belleğinde (shell.toml'a yazılmaz); config sıcaklığı değişince config geçerli. `blocked`
  (başka istemci): durum gösterilir, kendiliğinden tekrar denenmez; elle açma bir kez dener.
- **Kart** `widgets/quick/night_light.rs` (`night_light` panel widget'ı; 2026-10-15'ten beri varsayılan panelde
  YOK — aç/kapa ve durum `toggles` kutucuğunda, kaydırıcı için widget ekleme menüsü;
  `docs/design/quick-toggles.md` §2): anahtar satırı, alt metin durum ("Kapalı · açılış 20:00",
  "Açık · gün doğumunda kapanır", "Başka bir uygulama yönetiyor"), açıkken 1500–6000 K kaydırıcısı
  (100 K adım). `msg night-light status`: `{"supported","enabled","schedule","status","active",
  "temperature","current","override","next_change","location","today"}`. `location` yalnızca
  yapılandırılmış zamanlama `sun` iken (yoksa null): `{"source":"manual"|"timezone"|"none",
  "lat","lon"}` (`none`: konum yok, 20:00–07:00 kullanılıyor; lat/lon null). `today`: yerel bugünün
  `{"sunrise","sunset"}` unix saniye (biri yoksa null) ve `"polar":"day"|"night"|null`; konum
  yoksa null. Ayarlar (2026-10-13; onay) v1'de konumu salt okunur
  gösterir, elle konum `shell.toml`'dan; düzenlenebilir alanlar v2.

```toml
[night_light]
enabled = false        # ana anahtar (varsayılan kapalı)
temperature = 4000     # 1500..=6000 K
schedule = "sun"       # always | manual | sun
from = "20:00"         # manual; to <= from → gece yarısını aşar; from == to → tüm gün
to = "07:00"
# latitude = 41.0      # sun; ikisi de yoksa saat diliminden, o da yoksa 20:00–07:00
# longitude = 29.0
transition = 30        # 0..=120 dk, sınırın etrafında
```

## Büyüteç

Tasarım ve kararlar: `docs/design/magnifier.md` (t124; iş, kısayollar, varsayılan panel ve yeniden
yükleme kararları 2026-10-14). Hyprland `cursor:zoom_factor` (imleç çevresinde
yakınlaştırma); kabuk yalnızca değeri yönetir.

- **Servis** `src/services/magnifier.rs` (UI'dan bağımsız): saf durum makinesi `Magnifier`
  (çarpımsal adım, 1…`[magnifier] max` sınırı, `toggle` hafızası, sahiplik), komut metinleri
  (hyprlang `keyword cursor:zoom_factor <f>`, Lua `eval hl.config({cursor={zoom_factor=<f>}})`;
  dil her yazımda `j/status`'tan), `HyprControl` üzerinden `run`/`release`/`recover`/`reapply`,
  çökme işareti `Marker`.
- **Runtime** `app/magnifier.rs`: `nuxut-magnifier` thread'i; `msg magnifier` yanıtını o verir,
  görünümü `ShellMessage::Magnifier` ile modele (`ShellModel::magnifier`) koyar. Yazma yalnızca
  `SyncEnv::may_control()` iken (test örneği kullanıcının Hyprland'ine asla).
- **Sahiplik:** kabuk yalnızca kendi yazdığı büyütmeyi geri alır: kapanışta (sinyal, `msg quit`,
  `Shell` düşerken) canlı değer kabuğun son yazdığına eşitse 1'e döner; kullanıcı arada değiştirdiyse
  dokunulmaz. Çökme için `$XDG_RUNTIME_DIR/nuxut/magnifier-<imza>` (0600) — açılışta aynı kural.
- **Ayar yeniden yüklemesi:** Hyprland servisi `configreloaded`'ı `HyprlandEvent::ConfigReloaded`
  olarak iletir; canlı değer 1'e döndüyse kabuğun büyütmesi yeniden yazılır, başka bir değerse
  (kullanıcının ayarı) ezilmez ve sahiplik bırakılır.
- **Kart** `widgets/quick/magnifier.rs` (`magnifier`; varsayılan panelde YOK — widget ekleme
  menüsü ve Ayarlar › Erişilebilirlik): anahtar + "Açık · 2,5×", açıkken 0,25 adımlı kaydırıcı;
  komut gönderilemiyorsa gizli.
- **Kısayollar** (düzenden bağımsız): `magnifier_in` SUPER+ALT+Up, `magnifier_out` SUPER+ALT+Down
  (`bindel`), `magnifier_reset` SUPER+ALT+Escape (`bindl`).

## Ekran klavyesi

Tasarım, kararlar ve sınırlar: `docs/design/osk.md` (t125, 2026-10-14). Fare/dokunmayla
yazmak için alt kenarda klavye; tuşlar `zwp_virtual_keyboard_v1` ile odaklı istemciye gider.

- **Çekirdek** `src/services/osk/` (UI'dan bağımsız): `keys.rs` fiziksel tuş tablosu (evdev
  kodları, 5 × 15 birim), `keymap.rs` gömülü TR-Q/US keymap'leri (`assets/osk/*.xkb`, değiştirilmemiş
  xkeyboard-config) ve xkbcommon ile çıkarılan etiketler, `mod.rs` `Osk` durum makinesi: dokunuşları
  sırayla `Emit` (keymap → maskeler → tuş) çağrılarına çevirir; Shift/AltGr/Ctrl tek vuruş, Caps kilit.
- **Görünüm** `src/osk_view.rs` `KeyboardView`: yerleşim, isabet testi (bas → tut → bırak / kay /
  çık = bırak; tutamaçtan sürükleme), çizim (zemin framework `BlobField` panel geometrisi,
  `Interaction`/`Label`/`IconView`, tema token'ları). Yer `services/osk/place.rs` (yanaşık/yüzen,
  sürükleme durum makinesi; birim testli).
- **Yürütücü** `app/osk.rs`: `nuxut-osk` yüzeyi (Overlay, klavye boyutunda + gölge payı; iki kip
  de sol üst anchor + margin, exclusive zone -1 — pencereler itilmez, `[osk] reserve_space` ile
  yanaşıkken itilir; `KeyboardMode::None`; sürüklerken ve bırakınca yay boyunca framework
  `set_layer_placement` ile yerinde taşınır, yeniden kurulmaz; işaretçi tabanı compositor'a bağlı,
  `place::GrabFrame` — Hyprland tabanı kaynaktan (v0.56.2), canlı doğrulanmadı, `docs/design/osk.md`; başka monitöre bırakınca orada bir kez yeniden kurulur (`place::drop_target`); kip ve konum
  oturumda hatırlanır; karar 2026-10-16), sanal klavye çağrıları (framework `Context::virtual_keyboard_*`), düzen
  izleme (`[osk] layout = "auto"`: Hyprland `activelayout` → `HyprlandEvent::ActiveLayout` ve
  framework `keyboard_layout_changed`), kapanışta tuş/değiştirici sıfırlama.
- **Tek geçici yüzey kuralı istisnası:** klavye bir yaprak değil, girdi aygıtıdır; geçici yaprakla
  birlikte açık kalır ve yaprağın odak kapmasına eklenir (`Screen::focus_grab_extra`) — klavyeye
  dokunmak yaprağı kapatmaz, tuşlar yaprağın yazı alanına gider (başlatıcı, ağ parolası).
- **Tuş tekrarı** istemcide: tuş basılı tutuldukça basılı kalır (kullanıcının `repeat_rate`/`delay`).
- **Kart** `widgets/quick/osk.rs` (`osk`; varsayılan panelde yok), **kısayol** `osk_toggle` =
  SUPER+ALT+K, **komut** `msg osk toggle|show|hide [--output]` / `msg osk status` (osk-v1).
- **Sınır:** kilit ekranında yok (v2: `nuxut-lock` içinde); bar alttayken alt yapraklarla örtüşebilir.

```toml
[osk]
layout = "auto"     # auto | tr | us
key_height = 48     # 32..=72 px; genişlik 15 × 1,25 × key_height (ekrana sığar)
reserve_space = false  # true: yanaşık klavye pencereleri yukarı iter
```

## Komutlar (IPC)

`src/ipc.rs` saf komut modeli; satırlar asla shell'e verilmez. Runtime'ın tek örnek IPC'si
(`$XDG_RUNTIME_DIR/nuxut/nuxut-shell.sock`; ikinci `nuxut-shell` başlamaz) → `App::command` →
`ipc::parse_args` → `Shell::execute`. Hyprland örneği:
`hyprland.conf` örnekleri:

```ini
bind = SUPER, N, exec, nuxut-shell msg panel toggle right   # oturum + bildirim merkezi
bind = SUPER, C, exec, nuxut-shell msg panel toggle clock   # saat/takvim
bind = SUPER, B, exec, nuxut-shell msg bar toggle
bind = SUPER SHIFT, R, exec, nuxut-shell msg config reload

# ses/parlaklık tuşları (l: kilit ekranında da, e: basılı tutunca tekrarlar)
bindel = , XF86AudioRaiseVolume,  exec, nuxut-shell msg volume up
bindel = , XF86AudioLowerVolume,  exec, nuxut-shell msg volume down
bindl  = , XF86AudioMute,         exec, nuxut-shell msg volume mute
bindl  = , XF86AudioMicMute,      exec, nuxut-shell msg mic mute
bindel = , XF86MonBrightnessUp,   exec, nuxut-shell msg brightness up
bindel = , XF86MonBrightnessDown, exec, nuxut-shell msg brightness down
# medya tuşları shell'den geçmez (OSD'si yok): MPRIS'e doğrudan playerctl
bindl  = , XF86AudioPlay,  exec, playerctl play-pause
bindl  = , XF86AudioNext,  exec, playerctl next
bindl  = , XF86AudioPrev,  exec, playerctl previous
```

```text
nuxut-shell msg panel <toggle|open|close> <right|left|top|bottom|clock|launcher|sysmon> [--output <ad>]
nuxut-shell msg launcher <toggle|open|close|clipboard|emoji> [--output <ad>]   # clipboard/emoji: o kipte açar (o kipte açıksa kapatır, başka kipteyse geçer)
nuxut-shell msg osk <toggle|show|hide> [--output <ad>] | osk status   # ekran klavyesi (osk-v1 JSON)
nuxut-shell msg clipboard list | pick <id> | delete <id> | clear      # list: JSON v1 {v,enabled,available,items:[{id,kind,len,preview?,age_s}]}
nuxut-shell msg panel close-all [--output <ad>]          # sabitler dahil
nuxut-shell msg panel pin <right|left|top|bottom> [toggle|on|off] [--output <ad>]
nuxut-shell msg bar <toggle|show|hide> [--output <ad>]
nuxut-shell msg volume <up|down> [<adım%>] | volume set <0-150> | volume mute [toggle|on|off]
nuxut-shell msg mic mute [toggle|on|off]
nuxut-shell msg brightness <up|down> [<adım%>] | brightness set <0-100> [--output <ad>]
nuxut-shell msg config reload | config status | config undo | config undo-migration | config heal [--dry-run|--undo|--fix-typos|--ignore-typos] | ping | version | quit | help   # quit: düzgün kapan (çıkış 0; systemd birimi altında birim de durur — orada systemctl --user restart)
nuxut-shell msg hypr status | sync | add-source | undo   # durum JSON; yeniden üret+uygula; satırı bir kez zorla ekle; Nuxut satırını+dosyasını kaldır
nuxut-shell msg hypr restore-backup         # ADR-0007: en yeni ana ayar yedeğini geri koy (o anki dosya yedeklenir), yeniden yükle
nuxut-shell msg setup status | setup <done|skip> <1-65535>   # ilk açılış durumu JSON; Ayarlar sihirbazın sonucunu yazdırır
nuxut-shell msg look status | plan          # birleşik görünüm durumu / yazılacak parçalar (JSON, salt okunur)
nuxut-shell msg input status                # girdi: Hyprland etkin değerleri (getoption) + managed/overrides (JSON v1, salt okunur)
nuxut-shell msg clock refresh               # saat dilimi değişti: saat/takvim/gece ışığı 1,1 sn sonra yeniden hesaplanır (timedate1 sinyalinin yedeği; yanıt ok)
nuxut-shell msg defaults status | defaults set <rol> <masaüstü-kimliği> | defaults open <rol>   # rol varsayılanını başlat (yoksa ilk aday; temel katman kısayolları); varsayılan uygulamalar (JSON v1); set → {"ok":true,"snapshot","written"} ya da {"ok":false,"error":<kod>}
nuxut-shell msg session menu [--output <ad>]   # oturum kartının olduğu ilk açık kenar paneli (yoksa sağ); CTRL+ALT+Delete
nuxut-shell msg removable list              # çıkarılabilir birimler (JSON v1: id, name, size, mount_points)
nuxut-shell msg removable open|eject <id>   # bağla+aç / ayır+güç kes; id udisks blok adı (sdb1), bilinmeyen id hata; sonuç bildirimle (bildirim düğmeleri de bunu çağırır)
nuxut-shell msg bluetooth agent-status      # eşleştirme ajanı: {"registered":bool,"default":bool} (tek satır JSON; Ayarlar Pair öncesi sorar)
nuxut-shell msg images status               # görsel yalıtımı: kip (process|in_process|unavailable) + sayaçlar (JSON, içerik yok) + `wallpaper` (paylaşılan duvar kağıdı çözücüsü)
nuxut-shell msg night-light status | on | off | toggle | temperature <1500-6000>   # gece ışığı; status tek satır JSON, bkz. "Gece ışığı"
nuxut-shell msg magnifier status | in | out | reset | on | off | toggle | set <1-10>   # büyüteç; status tek satır JSON, bkz. "Büyüteç"
nuxut-shell msg wallpaper builtins          # yerleşik duvar kağıtları: [{id, source, name, variants, default, seed:{dark,light}}] (tek satır JSON, ad shell'in dilinde)
nuxut-shell msg power status                 # güç (JSON power-v1: upower, on_battery, battery, profiles, charge_limit)
nuxut-shell msg power charge-limit on|off|status   # pil şarj sınırı; on/off yanıtı UPower cevap verince: ok | error: <unavailable|unsupported|denied|failed>: …
nuxut-shell msg shot region|window|output|screen [--output <ad>] [--no-save] [--no-copy]   # JSON {"saved","copied"} | {"cancelled":true}; bkz. "Ekran görüntüsü"
nuxut-shell look apply                      # birleşik görünümü bir kez uygula / kapalıysa geri al (süreç içi, JSON)
nuxut-shell look revert                     # görünümü ayardan bağımsız Nuxut öncesine geri al (süreç içi, JSON; install.sh)
nuxut-shell hypr takeover | restore-backup   # ADR-0007: Nuxut'u göstermeyen ana Hyprland ayarını benzersiz yedekle devral (doğrulanmış) / en yeni yedeği geri koy (JSON; donanım/klavye/env ayarları user-migrated.conf'a taşınır, rapor + kabuk bildirimi msg hypr takeover-done; install.sh --hyprland-takeover, Hoş geldin onayı ve --hypr-restore; güncelleme kendiliğinden devralmaz)
nuxut-shell msg snapshot list [--setting <ayar>] [--limit <1-200>] | snapshot show <kimlik>
nuxut-shell msg snapshot restore <kimlik> [--force] [--setting <ayar>]   # ADR-0001 §9.3/§9.5; tek satır JSON, değer içermez
```
Snapshot komutları ayrı thread'de yanıtlanır (depo kilidi ≤ 5 sn); IPC yanıt penceresi ve `msg`
istemcisi 15 sn (`ipc::REPLY_TIMEOUT`).
`--output` yoksa panel komutları odaklı monitöre, bar komutları tüm monitörlere uygulanır.
Ses/parlaklıkta adım 1..=100 (varsayılan 5); `volume set` `[audio] max_volume`'e, parlaklık
dahili panelin taban değerine kırpılır (hata değil). `mute` argümansız = `toggle`. Ses/mikrofon
varsayılan cihaza uygulanır; parlaklık `--output` (yoksa odaklı monitör) adındaki DDC/CI
ekranına, o yoksa dahili panele. `--output` OSD'nin gösterileceği monitördür.

**OSD.** Varsayılan çıkış/giriş seviyesi, sessizliği ya da bir ekranın parlaklığı değişince
(komutla, tuşla ya da başka bir uygulamayla) bar'ın karşı kenarının ortasından çerçeveye
yapışık bir kapsül olarak büyür ve son değişimden `[osd] timeout_ms` sonra kapanır; aynı anda
tek monitörde. Monitör: (1) komutun `--output`'u, (2) parlaklıkta değişen ekranın monitörü
(harici ekranın çıktısı; dahili panelde `eDP`/`LVDS`/`DSI` monitörü), (3) odaklı monitör.
Gösterim kararı `app/osd.rs`'teki saf `OsdTracker`'da: yalnızca o kaynağın (varsayılan
çıkış/giriş, ekran) önceki değeri biliniyor ve yeni değer farklıysa. Başlangıç okuması,
PipeWire'a yeniden bağlanma (`[audio] max_volume` değişince ses servisi yeniden başlar),
varsayılan cihaz değişimi, ekran listesinin yeniden taranması ve aynı değerin tekrarı OSD
değildir. Ses kapsülü `[audio] max_volume`'e kadar ölçeklenir (%100 üstü gerçek yüzdeyle,
uyarı tonunda); mikrofon ve parlaklık %100. Sessizdeki ses/mikrofonda ikon, ray ve yüzde soluk
(framework `Osd::set_muted`, `DISABLED_CONTENT` opaklığı; seviye yine gösterilir — sessizi kaldırınca dönülecek değer).
Bar gizlenince bar kenarı yayla çerçeve kalınlığına iner, exclusion zone'u da küçülür.

## Pano geçmişi

Tasarım ve gizlilik kuralları: `docs/design/clipboard.md` (bağlayıcı). Yalnızca
`Selection::Clipboard` (birincil seçim `primary = true` ile); metin (UTF-8) ve küçük görsel
(png/jpeg); varsayılan yalnızca bellek. Kaynak framework data-control'üdür (`source.rs`).

- **Akış.** `App::data_control` olayı adaptörde (`DataControlSource::translate`)
  `SourceEvent::Offered { id, mimes, flags }` olur ve servis kuyruğuna gider (okuma servis
  thread'inde `DataControlHandle::read`). Framework `own`/`sensitive`/`truncated` → **okunmaz**.
  `filter::decide` MIME listesi + `Gate` (logind `LockedHint`, Hyprland `activewindow` sınıfı) +
  config ile karar verir: kapalı / kendi seçimimiz / kilitli / >64 MIME ya da >256 baytlık ad /
  `x-kde-passwordManagerHint` / adında `secret`-`password` / `exclude_apps` → **okunmaz**. Kabul
  edilen tek MIME `READ_TIMEOUT` (2 sn) ve tür sınırıyla (metin 256 KiB, görsel 8 MiB) okunur;
  art arda gelen tekliflerden yalnızca sonuncusu okunur. Okuma sürerken kilitlenirse saklanmaz.
- **Geçmiş.** En yeni başta; aynı bayt başa taşınır (kimlik korunur); öğe ve toplam (64 MiB)
  sınırında en eskiler düşer. İçerik `Arc` ile UI/IPC'ye paylaşılır; son sahip bırakınca sıfırlanır
  (`Blob`, `zeroize`). `Debug` yalnızca tür/uzunluk.
- **Arayüz.** `msg launcher clipboard` (varsayılan kısayol `SUPER+SHIFT+V`, eylem `clipboard`)
  başlatıcıyı pano kipinde açar: satır = ilk satır (≤ 200 karakter) ya da görsel küçük resmi
  (yalıtılmış çözücüde, `Untrusted`, en uzun kenar 96 px; işçi yoksa yer tutucu), alt metin
  göreli zaman · boyut. Enter/tık → `set_selection` + kapat (otomatik yapıştırma yok); Delete
  (imleç sonda) → sil; "Geçmişi temizle" `ConfirmButton`. Başlatıcı kapalıysa (`launcher.enabled =
  false`) pano kipi de açılmaz.
- **Disk** (`persist = true`): sürümlü sınırlı biçim, 2 sn birleştirme, `0600`/`0700`, atomik;
  `persist` kapanınca ya da temizlenince dosya silinir. Yazım ortamı kapalıysa (`apply_env`) yok.
- **İzleme yaşam döngüsü.** Compositor data-control sunmuyorsa servis başlamaz (`available:
  false`, tek bilgi satırı). `enabled = false` → `data_control_stop` (tür listesi bile gelmez).
  `Stopped` / `NoSeat` → geçmiş korunur, `available: false`, `Restart` ile 1 sn → 5 dk üstel
  yeniden deneme (60 sn kararlı çalışma sayacı sıfırlar); `Unsupported` yeniden denenmez.
  Gerekçe: `docs/design/clipboard.md` "aşama 2".
- **Test kaynağı.** `clipboard-demo` cargo özelliği (ürüne girmez) + `NUXUT_CLIPBOARD_DEMO=1`:
  sabit tekliflerle başlatıcı pano kipini izole sway'de gösterir; gerçek panoya dokunmaz.

### VM pano köprüsü

Tasarım `docs/design/vm-clipboard.md`, araştırma `docs/research/vm-clipboard.md` (2026-10-16,
kullanıcı isteği). Hyprland Xwayland panosunu yalnız bir X11 penceresi odaktayken eşitler; VM
konuk ajanları (spice-vdagent, vmtoolsd, VBoxClient) panoyu yalnız X11 ile konuşur ve penceresizdir.
`services/clipboard/x11/` (thread `nuxut-x11-clip`, x11rb) bunu olay tabanlı köprüler:

- **Kip** `[clipboard] x11_bridge = "auto" | "on" | "off"`. `auto`: hipervizör + konuk ajanı süreci
  (ilk 5 dk seyrek tarama, sonra durur). Köprü Hyprland'in odak korumasını kaldırdığı için VM
  dışında kapalı. `DISPLAY` yoksa iş yok. Devreye girince (`BridgeEvent::Active`) geçmiş kapalı
  olsa da data-control izlenir.
- **Yetki** (2026-10-16, güvenlik düzeltmeleri): yalnız yerel `DISPLAY` ve bu kullanıcının
  Xwayland'i (`SO_PEERCRED` uid + `comm`); `auto`'da panoyu yalnız bu kullanıcının konuk ajanı okur
  ve koyar — isteyen/sahip **penceresinin** XRes PID'i konuk ajan süreci olmalı (`/proc` sahibi + adı);
  odak temelli izin yok (isteyen pencere istemcice seçilir, odak isteği gönderene bağlanamaz;
  odaktaki X11 uygulamalarını Hyprland XWM eşitler). Her istekte önce kilit, sonra yanıt penceresi
  ve özelliği (`WM_*`/`_NET_*`/`RESOURCE_MANAGER` yazılmaz) denetlenir; konuk ajanın okumadığı yanıt
  özelliği 2 sn sonra silinir. `on` korumayı herkes için kaldırır.
- **X11 → Wayland:** XFixes sahip değişimi → `TARGETS` → metin + tek görsel + ≤ 4 ek tür (≤ 16 MiB,
  INCR alımı) → `set_selection` (işaret `application/x-nuxut-x11-bridge`). Bu işaretli kendi teklifimiz
  pano geçmişine girer (geçmiş onu `own` saymaz).
- **Wayland → X11:** teklif → X11 `CLIPBOARD`'ı alırız, türleri tembel sunarız (içerik yalnız X11
  istemcisi isteyince okunur; INCR gönderimi yok, tek isteğe sığmayan reddedilir).
- **Döngü önleme:** işaret türü, kendi penceremiz, compositor XWM istemcisi (`WM_S<n>` sahibiyle
  aynı istemci tabanı), compositor'ın X11 → Wayland yansıtması (1 sn + tür alt kümesi), aynı içerik özeti.
- **Gizlilik:** hassas teklif (parola ipucu, `secret`/`password`, kırpılmış liste) iki yönde de
  taşınmaz ve okunmaz; oturum kilitliyken ya da `exclude_apps` uygulaması etkinken geçmişle aynı
  `Gate` iki yönü de durdurur (kilit ekranı ayrı süreç: framework'ün kilit kuralı kabukta işlemez;
  güvenlik düzeltmesi 2026-10-16); içerik loglanmaz, tamponlar `Zeroizing`.
- **Dayanıklılık:** hedef ≤ 64, ad ≤ 256 bayt; X11 protokol hatası yalnız o isteği düşürür;
  bağlantı kopunca 1 sn → 5 dk geri çekilmeyle yeniden bağlanılır.
- **Kullanıcı betiği** (`wl-paste` + `xclip`/`xsel` köprüsü başlatan `exec-once`; ör.
  `~/hypr-bridge.sh`): çalışma anında, yalnız Nuxut köprüsü Xwayland'e ve data-control'e bağlıyken
  bir kez yorumlanmış bloğa çevrilir (`services/clipboard/takeover.rs`, snapshot
  `hypr.clipboard_bridge`); betik silinmez, çalışan sürece dokunulmaz; geri alma `msg hypr undo`.
  Gerekçe: iki köprü birbirini besler ve betik hassas teklifi (KeePassXC parolası) ana makineye taşır.

## Ekran görüntüsü

`nuxut-shell msg shot region|window|output|screen [--output <ad>] [--no-save] [--no-copy]`;
varsayılan kısayollar Print / Shift+Print / Alt+Print (`[keybinds] shot_region|shot_output|
shot_window`). Tasarım ve gerekçe: `docs/design/screenshot.md`.

- **Yakalama** `grim -t ppm` alt süreciyle (compositor'ın wlr-screencopy / ext-image-copy-capture
  desteği; framework'te screencopy yok). `output`: `grim -o <ad>`; `window`: Hyprland
  `j/activewindow` geometrisi → `grim -g`; `screen`: çıkış başına paralel `grim -o`, düzene
  göre kendi birleştirmemiz (tuval ölçeği çıkışların en büyüğü, 1/120 adımına oturtulur; düşük
  ölçekli çıkış çift doğrusal büyütülür, kaplanmayan alan şeffaf).
- **region**: önce tüm çıkışlar yakalanır (dondurulmuş kare), sonra çıkış başına
  `nuxut-screenshot` Overlay yüzeyi açılır; kare fiziksel çözünürlükte doku olarak tam ekran
  çizilir, seçim dışı temanın gölge rengiyle karartılır. Sürükle = yeni seçim, içinden sürükle =
  taşı, kenar/köşe = yeniden boyutlandır, ok tuşları 1 px (Shift 10 px), Enter/Space onay,
  Esc/sağ tık iptal; seçim yokken tıklama imlecin altındaki pencereyi (Hyprland `j/clients`,
  görünür workspace'ler; tam ekran > yüzen > döşenmiş, küçük alan önce), yoksa çıkışı onaylar.
  Seçim tek çıkış içinde; başka çıkışta başlayan seçim öncekini kaldırır. Onayda katmanlar hemen
  kalkar; kesim dondurulmuş kareden yapılır (görülen = kaydedilen; katman görüntüye girmez).
  Ölçek görüntüden türetilir (`görüntü / mantıksal boyut`): kesirli ölçekte de piksel doğru.
- **Teslim** (arka plan thread'i): PNG (opaksa RGB), `[screenshot] dir` (boş:
  `user-dirs.dirs` `XDG_PICTURES_DIR`, yoksa `~/Pictures`, altında yerelleştirilmiş klasör),
  ad `Ekran görüntüsü YYYY-MM-DD HH-MM-SS.png`, çakışmada `-2`…; dizine göre `openat(O_CREAT |
  O_EXCL | O_NOFOLLOW)` (üzerine yazma ve bağlantı izleme yok). `wl-copy --type image/png`.
  Bitince shell'in kendi popup bildirimi: "Aç" (`xdg-open`) ve "Klasörde göster"
  (`org.freedesktop.FileManager1.ShowItems`, yoksa klasörü açar); düğmeler `shot open|reveal
  <n>` (bellekte son 16 kayıt). Bildirimde küçük resim önizlemesi: teslim thread'inde
  `geometry::thumbnail` ile en uzun kenarı 256'ya (`IMAGE_MAX_SIDE`) alan ortalamasıyla
  küçültülür (`downscale_area`: kesirli örtüşmeli kutu süzgeci, alfa ağırlıklı; küçük görüntü
  büyütülmez), `Delivered.preview` ile ana thread'e taşınır, `InternalNotice::with_image`.
  Yalnızca `screenshot.notify` açıkken üretilir; başarısızlık bildiriminde görsel yok.
- **Yanıt**: tek satır JSON `{"saved":<yol>|null,"copied":bool}` (kısmi hatada `warning`),
  iptalde `{"cancelled":true}`, hata `error: …`. Tek oturum (sürerken yeni `shot` → `busy`).
  Seçim IPC yanıt penceresinden (15 sn) uzun sürerse istemci yanıtı alamaz; iş yine yapılır.

## Portal arka ucu (`xdg-desktop-portal-nuxut`)

Çalışma alanının `portal/` üyesi; UI'sız küçük ikili (`$PREFIX/lib/nuxut/`), xdg-desktop-portal
D-Bus ile başlatır. Tasarım ve kararlar: `docs/design/portal.md`.

- `org.freedesktop.impl.portal.Settings` (sürüm 2), yalnızca `org.freedesktop.appearance`:
  `color-scheme` (`Theme::mode`: koyu 1 / açık 2), `accent-color` (`Theme::colors.primary`,
  sRGB 0..1), `contrast` (0), `reduced-motion` (`Theme::motion.enabled` yanlışsa 1). Değerler
  runtime'ın `ThemeLoader`'ından (framework.toml + palette.toml); birleşik görünüm kapalıysa arka
  uç saydam (hiçbir anahtar; gtk arka ucu → dconf).
- `framework.toml` ve `palette.toml` dizinleri inotify ile izlenir (`poll(2)`, boşta uyanma yok);
  yalnızca değeri değişen anahtar için `SettingChanged`.
- `org.freedesktop.impl.portal.FileChooser` (`OpenFile`, `SaveFile`, `SaveFiles`; `portal/src/chooser/`):
  arayüz bu süreçte, görünüm istek başına ayrı `nuxut-filechooser` süreci (portal UI'sız kalır,
  boşta iş yok; 2026-10-15). Sözleşme stdin/stdout tek satır JSON, yollar yüzde kodlu
  bayt; stdin'in kapanması iptal (`Request.Close` ya da portal ölümü → yetim seçici yok), 2 sn
  sonra `SIGKILL`. Portal sonucu yeniden doğrular (mutlak, `..`'sız, var/klasör kuralları, seçim
  ve filtre sunulanlardan) ve yalnızca seçilen yolları `file://` URI olarak döner; yollar ve
  başlıklar loglanmaz. Eşzamanlı ≤ 8 seçici. Tasarım: `docs/design/file-chooser.md`.
- Seçim: kabuk Nuxut oturumunda Hyprland ortamına `XDG_CURRENT_DESKTOP=Nuxut:<önceki>` yazar
  (`services::portal_env` → `SyncRequest::session_env` → üretilen `nuxut.lua`/`nuxut.conf` `env`,
  imleç ortamından önce; yalnızca `$XDG_DATA_HOME/xdg-desktop-portal/portals/nuxut.portal` varsa).
  environment.d değil: Hyprland açılışta kendi `XDG_CURRENT_DESKTOP`'unu systemd/D-Bus ortamına
  aktarır. xdg-desktop-portal `$XDG_CONFIG_HOME/xdg-desktop-portal/nuxut-portals.conf`'u okur
  — sabit değil: Nuxut olmasaydı Hyprland için seçilecek dosyanın (xdp 1.22 arama sırası) kopyası,
  tek fark `Settings` başına `nuxut` (`portal/src/portals_conf.rs`; kullanıcının `FileChooser = kde`
  gibi seçimleri korunur). Kullanıcı o dosyayı değiştirirse `xdg-desktop-portal-nuxut
  --write-portals-conf` (ya da install.sh) yeniden üretir; otomatik tazeleme v1 dışı
  (docs/design/portal.md "Seçim dosyası"). Kullanıcının
  `hyprland-portals.conf`'una dokunulmaz.
- xdg-desktop-portal ortamı ve kayıtları yalnızca açılışta okur: kurulum/güncelleme etkisi bir
  sonraki girişte (çalışan portal yeniden başlatılmaz).
- **Bilinen sınır — uwsm (doğrulanmadı, 2026-10-13; bu makinede uwsm yok):** seçim Hyprland
  config'teki `env XDG_CURRENT_DESKTOP` satırının systemd/D-Bus etkinleştirme ortamına
  ulaşmasına dayanır. uwsm ile başlatılan oturumda ortamı uwsm, Hyprland'den önce aktarır;
  config `env` satırı xdp'ye ulaşmazsa xdp `Hyprland` görür → `hyprland-portals.conf` seçilir →
  `Settings` gtk arka ucundan (dconf) gelir. İşlev kaybı yok, yalnızca Nuxut görünüm portalı
  devreye girmez. Elle çözüm: `~/.config/uwsm/env-hyprland` içine
  `export XDG_CURRENT_DESKTOP=Nuxut:Hyprland`; install.sh v1 uwsm dosyalarına yazmaz.

## Ekran paylaşım seçicisi (`nuxut-sharepicker`)

xdph'in `screencopy:custom_picker_binary` sözleşmesine uyan seçici; `nuxut-shell` paketinin ikinci
ikilisi (`$PREFIX/lib/nuxut/`). Tasarım, sözleşme (kaynağı: xdph 1.4.1 ikili analizi) ve
doğrulanmamış noktalar: `docs/design/screenshare-picker.md`.

- Sözleşme katmanı `services::sharepicker` (saf): `XDPH_WINDOW_SHARING_LIST` ayrıştırma (boyut
  sınırlı, bozuk girdi atlanır), çıktı `[SELECTION]<r?>/screen:<çıkış>`, `window:<id>` ya da
  `region:<çıkış>@<x>,<y>,<genişlik>,<yükseklik>` (çıkışa göre mantıksal px).
  stdout yalnızca bu satır; iptal = boş stdout (çıkış 1).
- Görünüm `sharepicker_view`: odaklı çıkışta `Overlay` + `Dialog`; sekmeler Ekran / Pencere / Bölge;
  bölge seçimi ekran görüntüsünün durum makinesiyle (`services::screenshot::selection`), her
  çıkışta canlı (dondurulmamış) karartılmış katman. Yazı-önü kapısı 400 ms; 180 sn
  etkileşimsizlikte iptal (xdph seçiciyi senkron bekler).
- Kurulum: `dist/xdph/xdph-picker.sh` `xdph.conf`'a işaretli blok (yedekli, idempotent; kullanıcının
  kendi seçicisi ya da sembolik bağ varsa dokunmaz). `--no-sharepicker` atlar.

## Kilit ekranı (`nuxut-lock`)

Projenin güvenlik açısından en kritik parçası. Sahibi. Tasarım onaylandı (2026-10-04); uygulama sırası: `nuxut-pam` (yapıldı) → durum makinesi → F1 gelince arayüz → shell tetikleyicisi.

### Tehdit modeli

**Korunan:** kilitliyken oturumun içeriği (pencereler, bildirimler, medya, pano) ve oturumun
kendisi. **Saldırgan:** klavye/fare/ekran başındaki, şifreyi bilmeyen kişi; çıkış takıp çıkaran,
uykuya sokup uyandıran, klavye düzenini değiştiren, rastgele tuş/IME girdisi üreten, kilit
ekranını çökertmeye çalışan.

**Kapsam dışı (her Wayland kilidi için aynı):** kullanıcının kendi uid'iyle çalışan kötü amaçlı
kod — kilit sürecini öldürebilir, Hyprland `allow_session_lock_restore` açıksa kendi "kilidini"
alıp açabilir, `ptrace` sınırları dışında belleği okuyabilir. Root, compositor'un kendisi ve
çekirdek güvenilir sayılır. Bu sınırlar belgede ve ayarlarda açıkça yazılır; "same-uid'e karşı
koruyoruz" iddiası yapılmaz. Yine de ucuz sertleştirmeler uygulanır (aşağıda).

**Temel özellikler (testle korunur):**
1. *Fail-secure:* kilit istemcisi çökerse, öldürülürse, panic ederse ya da protokol hatası alırsa
   oturum **kilitli kalır** (ext-session-lock garantisi; Hyprland'de "lockdead" ekranı). Hiçbir
   hata yolu `unlock_and_destroy` göndermez; yalnızca başarılı PAM sonucu gönderir.
2. *Erken içerik yok:* kilit "başarılı" ancak compositor `locked` olayını gönderince sayılır
   (protokol: tüm çıkışlarda kilitli kare sunulmadan gönderilmez). `finished` önce gelirse kilit
   başarısızdır, çıkış kodu ≠ 0, tetikleyen tarafa hata döner.
3. *Boş çıkış yok:* kilitliyken eklenen her çıkışa hemen kilit yüzeyi açılır; kaldırılanın yüzeyi
   yok edilir. (Yüzeyi olmayan çıkışı compositor zaten karartır; yine de içerik orada çizilir.)
4. *Şifre sızmaz:* şifre hiçbir log'a, `Debug` çıktısına, IPC'ye, D-Bus'a, IME'ye (surrounding
   text) gitmez; kopyaları en aza indirilir ve `zeroize` ile sıfırlanır.

### Süreç mimarisi: ayrı `nuxut-lock` ikilisi

Kilit, shell'den **ayrı bir süreçtir**. Gerekçe:
- *Çökme yalıtımı, iki yönde:* shell çökerse (tepsi, bildirim, takvim kodu — geniş yüzey) kilit
  sürer, oturum açılmaz; kilit çökerse compositor kilitli tutar. Shell içinde olsaydı tepsideki
  bir D-Bus hatası kilidi "lockdead"e düşürürdü.
- *Küçük saldırı yüzeyi:* kilit süreci tepsi/bildirim/DBusMenu gibi dış girdi işlemez; yalnızca
  klavye/işaretçi, logind ve (isteğe bağlı) fprintd ile konuşur.
- *Ayrı bellek:* şifre shell'in adres alanına hiç girmez; kilit süreci `PR_SET_DUMPABLE=0` ile
  çekirdek dökümü ve aynı uid'den `/proc/<pid>/mem`/ptrace erişimini kapatır.
- *Bağımsız yaşam:* kilit, shell çalışmasa da hypridle/`loginctl lock-session` ile kullanılabilir.

Konum: aynı repoda Cargo workspace üyesi `lock/` (ikili `nuxut-lock`) + FFI'yi yalıtan küçük
crate `crates/nuxut-pam`. `unsafe` yalnızca `nuxut-pam` içinde (her blok `// SAFETY:`);
`nuxut-lock` ve shell `unsafe_code = "forbid"` kalır. Kök `Cargo.toml`'a `[workspace]` eklemek
karardır (alternatif: ayrı repo — paylaşılan tema/IPC kodu yüzünden önerilmez).

Yaşam döngüsü tek atımlıktır: başla → kilitle → doğrula → aç → çık. Boşta yerleşik süreç yok
(GL context'i ve fontları kilit dışında bellekte tutmamak için).

```text
nuxut-lock                         # kilitle; zaten kilitliyse (tek örnek) hemen 0 ile çık
nuxut-lock --ready-fd <n>          # 'locked' olayından sonra fd'ye "locked\n" yaz, kapat
nuxut-lock msg status              # "locked" | "pending" | (çalışmıyorsa) hata
```
Tek örnek: framework `ipc::try_single_instance("nuxut-lock")`; `Secondary` → çalışan örneğe
`status` sorar, 0 ile çıkar (çift tetikleme zararsız). `NUXUT_LOCK_INSTANCE=<ad>` (yalnızca
`[a-z0-9_-]`, ≤ 16; geçersizse hata) IPC adını `nuxut-lock-<ad>` yapar: testler kullanıcının
kilidine komut göndermez.

**Canlı test:** yalnızca izole sway'de (`tools/test-compositor.sh`). `--features fake-auth`
(yalnızca debug derlemesi; sürümde `compile_error`) ile `NUXUT_LOCK_FAKE_SECRET` sahte
doğrulayıcıyı — yalnızca `NUXUT_LOCK_INSTANCE` de tanımlıysa (yoksa yok sayılır, gerçek PAM) —
açar ve sistem bus'ını (logind `SetLockedHint`, fprintd) **kapatır** — gerçek
oturumun `LockedHint`'i ve parmak izi cihazı teste karışmaz. Test betiği `flock -o` ile
çalışır: kilit süreci ortak test kilidinin fd'sini miras almasın. Çıkış kodları: 0 kilitlendi ve açıldı / zaten
kilitli; 1 compositor reddetti (`finished`) ya da protokol yok; 2 başlatma hatası.

### Durum makinesi

```text
Starting ──lock_session()──▶ Pending ──locked──▶ Locked ──PAM ok──▶ Unlocking ──anim──▶ unlock_and_destroy → çık(0)
                               │                    │                    (≤ 400 ms, motion.effects; zaman aşımı yedeği)
                               └──finished──▶ çık(1)  └──finished──▶ unlock_and_destroy → çık(0)  (compositor kendi açtı)
```
- `Pending` sırasında her çıkış için kilit yüzeyi **hemen** yaratılır ve ilk karesi çizilir
  (compositor ara boş kare göstermesin). `locked` gelmeden "kilitlendi" denmez, `--ready-fd`
  yazılmaz, logind `SetLockedHint(true)` çağrılmaz.
- `Locked`'ta SIGTERM/SIGINT/SIGHUP **yok sayılır** (log). Yalnızca `Pending`'de sinyal temiz
  çıkış (`destroy`, kilit hiç olmadı). Panic = abort (release profili) → compositor kilitli tutar.
- Açma: PAM başarı → secret sıfırlanır → içerik soluklaşır → `unlock_and_destroy` + flush →
  `SetLockedHint(false)` (yalnızca `unlock_session` `Ok` dönerse; reddedilen açmada oturum kilitli
  kalır, ipucu `true` kalır) → çık. Animasyon başarısızsa (kare gelmiyor: DPMS kapalı) 400 ms timer.

### Kimlik doğrulama (PAM)

`crates/nuxut-pam`: libpam'e doğrudan, **kendi minimal FFI'miz** (`pam_start`, `pam_authenticate`,
`pam_setcred(PAM_REFRESH_CRED)`, `pam_end`, `pam_strerror`, test için `pam_start_confdir`;
`getpwuid_r`). Değerlendirme:

| Seçenek | Lisans | Neden değil / neden |
|---|---|---|
| `pam-client` 0.5 | MPL-2.0 | `pam-sys` 1.0-alpha (bindgen → derlemede libclang); yanıt tamponlarının sıfırlanması bizim denetimimizde değil |
| `pam` 0.8 | MIT/Apache | Bakımı zayıf, alpha `pam-sys`, şifreyi sıfırlanmayan `CString`'de tutar |
| `nonstick` 0.1 | MIT | Çok genç (0.1.x), az kullanıcı; güvenlik kritik yolda olgunluk istiyoruz |
| **Kendi FFI** (~200 satır) | — | 6 fonksiyon; conversation'da şifrenin tek kopyasını doğrudan `malloc`'lı tampona yazarız; bindgen yok, ek crate yalnızca `libc` (ağaçta zaten var) + `zeroize` |

Kurallar:
- **Kullanıcı adı** `getpwuid_r(getuid())`'den; `$USER` kullanılmaz.
- **Servis:** `/etc/pam.d/nuxut-lock` varsa `nuxut-lock`, yoksa `login` (uyarı loglanır). Dosya
  yokken "nuxut-lock" ile `pam_start` PAM'ı `other`'a (Arch'ta `pam_deny`) düşürür → kullanıcı
  kalıcı kilitli kalırdı; bu yüzden varlık kilitlemeden **önce** denetlenir. Paketle gelecek
  dosya: `auth include login` (hyprlock'un dosyasıyla aynı). Kurulumu root ister → paket/kullanıcı
  adımı.
- Yalnızca `pam_authenticate` (+ sonucu yok sayılan `pam_setcred(PAM_REFRESH_CRED)`; Kerberos vb.
  bilet yenileme). `pam_acct_mgmt` **çağrılmaz**: hesap politikası (nologin, pam_time, süre
  dolumu) oturum açılışına aittir; kilitte uygulanması kullanıcıyı kendi oturumuna kilitler
  (swaylock/hyprlock ile aynı karar).
- **Conversation:** `PROMPT_ECHO_OFF` → şifre; `PROMPT_ECHO_ON` → boş yanıt (kullanıcı adı zaten
  verildi); `ERROR_MSG`/`TEXT_INFO` → arayüze iletilen mesaj listesi (≤ 4 mesaj, her biri ≤ 256
  bayt, kontrol karakterleri atılır; pam_faillock'un "hesap kilitlendi" mesajı buradan görünür).
  Aynı istek içinde ikinci kez `ECHO_OFF` istenirse (OTP/çok adımlı) bu sürümde başarısız
  döner ("desteklenmeyen istem"), şifre iki kez verilmez. Yanıt dizisi `calloc`, yanıt metni
  `malloc` (PAM `free` eder; Linux-PAM modülleri `_pam_overwrite` ile siler). Hatalı/null girdi
  → `PAM_CONV_ERR`, panic FFI sınırını geçmez (`catch_unwind` yok; conversation panic etmeyen
  kodla yazılır ve `panic = "abort"`).
- **Thread:** her deneme ayrı bir `nuxut-auth` thread'inde (UI donmaz; pam_unix'in 2 sn hata
  gecikmesi, ağ modülleri). Aynı anda tek deneme. Thread sonucu `MessageSender` ile döner;
  sonuç gelmeden (ör. 60 sn takılı modül) yeni deneme başlatılmaz, alan "doğrulanıyor" kalır.
- **Artan gecikme** (PAM'in kendi gecikmesine ek): 3 başarısızdan sonra alan kilitlenir —
  2, 4, 8, 16, 30 sn (üst sınır), geri sayım gösterilir. Sayaç bellekte; başarıda sıfırlanır.
  faillock'un hesap kilidi PAM mesajıyla gösterilir ("çok sayıda hatalı deneme — hesap X dk
  kilitli"; metin PAM'in kendi çevirisiyle gelir): mesaj varsa durum satırında "Yanlış şifre"
  yerine o gösterilir. faillock kilitliyken PAM doğru şifreyi de reddeder; bu bilgi
  kullanıcıdan saklanmaz. Otomatik yeniden deneme yapılmaz.
- **Başarı yalnızca** `pam_authenticate == PAM_SUCCESS`. Diğer her kod (`PAM_CONV_ERR`,
  `AUTHINFO_UNAVAIL`, `pam_start` hatası, thread panik/kanal kopması, `MAXTRIES`, bilinmeyen kod)
  "başarısız" demektir — eşleme `match`'in `_ => Failed` koluyla, testli.
- Test edilebilirlik: UI `trait Authenticator` arkasından konuşur (`Pam` ve testte `Fake`).
  Gerçek FFI testi kullanıcı şifresi olmadan: `pam_start_confdir` ile geçici dizinde servis
  dosyaları — `pam_permit`, `pam_deny` ve `pam_exec.so expose_authtok <betik>` (conversation'dan
  gelen şifreyi sabit test şifresiyle karşılaştıran kısa POSIX betik). `confdir` yalnızca
  `#[cfg(test)]`/`testing` özelliğinde derlenir; üretim ikilisinde yok.

### Ekran okuyucu (2026-10-17; framework ADR-0011 madde 8)

Kilit yüzeyleri AT-SPI ağacı yayımlar (`set_a11y_lock_surfaces(true)`, `lock_session`'dan önce);
**AT yalnız okur, kilitliyken hiçbir şey sızmaz.** Ağaçta yalnız saat, tarih, ad, parola alanı
(`PasswordInput`, açıklama yalnız boş/dolu; değer, uzunluk ve maske yok; metin parçası yok →
AT-SPI `EditableText`/`Text`/`Action` ve text-changed yok) ve arındırılmış durum metni (canlı
bölge; PAM/faillock mesajı, kalan deneme, geri sayım yok). Kabuk kilitliyken (`LockedHint`)
ağacını askıya alır. `accessibility_action` her AT eylemini sessizce reddeder; a11y modülleri
hiçbir `RUST_LOG`la loglanmaz. Kilidi açma yalnız gerçek Enter ile (açma düğmesi yok), boş
parolada Enter PAM denemesi başlatmaz (faillock). Ayrıntı: `docs/design/screen-reader.md`
"Kilit ekranı".

### Parmak izi (isteğe bağlı, varsayılan: fprintd varsa açık)

fprintd'ye sistem bus'ında doğrudan D-Bus (`net.reactivated.Fprint`): `Manager.GetDefaultDevice`
→ `Device.Claim(user)` → `VerifyStart("any")` → `VerifyStatus(result, done)` sinyali. Şifreyle
**paralel** (kendi thread'i); `verify-match` → açma yolu (PAM ile aynı `Unlocking`). Başarısız
okuma sayısı sınırlı (varsayılan 3, sonra durur, şifre kalır). Sinyal yalnızca hedef
servisin sahibinden kabul edilir (zbus proxy eşleşmesi). Açma, kilit kapanırken ve uykuya
girerken `VerifyStop` + `Release`. fprintd yoksa/cihaz yoksa sessizce kapalı.

### Şifrenin bellekteki yolu

```text
klavye ─▶ KeyEvent.text (framework, tek karakter) ─▶ TextInput (gizli kip, framework)
      ─Enter▶ SecretBuf (zeroize + mlock, sabit kapasite 512 B = PAM_MAX_RESP_SIZE, büyümez) ─move▶ auth thread
      ─▶ conversation: malloc'lu C dizgisi (PAM free eder) ; SecretBuf drop → sıfırlanır
```
- `SecretBuf`: `Zeroizing<Vec<u8>>`, kapasite önceden ayrılır (yeniden tahsis = eski kopyanın
  serbest belleğe kalması), `mlock` (swap'e gitmesin; başarısızsa uyarı, devam), `Debug` →
  `SecretBuf(<gizli>)`, `Clone` yok, uzunluk bile loglanmaz.
- Gönderimde ve hatada TextInput **hemen** sıfırlanır (framework talebi F3).
- Kalan risk (belgelenir): klavye olayının tek karakterlik `String`'i ve xkbcommon iç
  tamponları sıfırlanmaz; PAM modüllerinin kendi kopyaları modülün sorumluluğundadır.

### Arayüz

Tüm bileşenler framework'ten, tüm değerler temadan (tonal dil, yay hareketi; sabit renk yok).
- Katmanlar (alttan üste): duvar kağıdı (`nuxut-image` `Wallpaper`, yalıtılmış çözücüyle — aşağıda;
  `lock.toml wallpaper`, yoksa shell.toml `[wallpaper] path` — yerleşik varsayılan dahil, masaüstüyle
  aynı kaynak —, o da `""` ise `palette.toml [meta] image`) + bulanıklık (`Wallpaper::set_blur`,
  32 px, kaynaktan önce) + karartma → saat
  (Display) ve tarih (yerel ayar) → avatar (`Avatar`: `~/.face`, yoksa AccountsService ikonu,
  yoksa baş harfler) + gerçek ad (GECOS, yoksa kullanıcı adı) → şifre alanı (`TextInput`,
  `password(true)`, `with_max_bytes(512)`) → durum satırı (Caps Lock, PAM mesajı, geri sayım,
  klavye düzeni adı yalnızca birden çok düzen varsa).
- **Görseller yalıtılmış işçide** (`lock/src/images.rs`, framework "Duvar kağıdı yalıtımı";
  karar 2026-10-13): kilit süreci PAM sırrını tutar, kullanıcı dosyası (duvar kağıdı,
  `~/.face`) bu adres alanında ayrıştırılmaz. `LockImages::new()` süreç başına tek
  `Sandbox::new(SandboxConfig::wallpaper())` (tembel; tek işçi, boşta süreç yok) ve üstünde tek
  `WallpaperDecoder` (`linger: ZERO`: kuyruk boşalınca kare bırakılır, aynı turdaki çıkışlar yine
  paylaşır) kurar; bütün `OutputView`'lar aynı çözücüyü `Wallpaper::new`'a verir. **Avatar** aynı
  `Sandbox`'tan: `open_user_file` (normal dosya, `O_NONBLOCK`) → `decode_file(file_key("avatar",
  yol), …)`, `Limits::WALLPAPER`, `TargetSize::cover_crop(256, 256)`, yalnızca ilk kare; eskiden
  süreç içinde `nuxut_image::decode_file` ile çözülüyordu (wallpaper-sandbox §9 bulgusu kapandı).
  Hata (`Unavailable`/`Protocol`/zaman aşımı/bozuk dosya): panik yok; duvar kağıdında `surface`
  zemini (hata bir kez `warn`), avatarda baş harfler (`debug`). İşçi yoksa framework S1:
  PNG/JPEG/WebP/GIF/QOI süreç içinde ilk kare (süreç başına tek uyarı), SVG/JXL/AVIF `Unavailable`.
- **Çoklu monitör:** her çıkışa yüzey; giriş alanı yalnızca klavye odağındaki yüzeyde
  (`keyboard_focus(true)`), odak gelmeden işaretçinin olduğu çıkışta, o da yoksa ilk çıkışta.
  Diğerleri saat + tarih. Şifre durumu tek (süreç başına), yalnızca çizildiği yer değişir.
- **IME:** `set_text_input` `TextPurpose::Password` (content hint `sensitive_data|hidden_text`);
  surrounding text hiç gönderilmez (TextInput gizli kipte zaten boş). Preedit gizli kipte
  maskeli gösterilir.
- **Caps Lock** uyarısı (framework talebi F2), **sarsılma:** hatalı şifrede alan yatay
  `spatial_fast` yayıyla ofset darbesi alır (taşan yay doğal sönümlü sarsıntı verir; bezier
  bounce değil), `error` rengi `effects` ile geçer.
- "Doğrulanıyor": alan içi `Spinner`; "kilit açılıyor": içerik + karartma `effects` ile solar.
- **Gizlilik varsayılanları:** bildirimler, medya kontrolleri, pano görünmez
  (`lock.toml show_media = false`, `show_notifications = false`; açılırsa yalnızca uygulama adı
  ve sayı — içerik asla).
- Klavye: Enter gönder, Escape alanı temizle, Ctrl+U temizle; yapıştırma ve kopyalama yok.
- Boşta sıfır iş: saat dakikada bir; animasyon yokken kare yok.

`~/.config/nuxut/lock.toml` (yoksa varsayılanlar; bozuksa varsayılanlar + uyarı — bozuk config kilidi
asla engellemez):
```toml
wallpaper          = ""      # boş: shell.toml [wallpaper] path (varsayılan nuxut:pair-flight), o "" ise palette.toml [meta] image (dosya ya da nuxut: adresi), o da yoksa düz surface; "nuxut:<id>[/dark|light]" yerleşik
blur               = true    # duvar kağıdı bulanıklığı (32 mantıksal px); dim ayrıca uygulanır
dim                = 0.55    # 0..=0.9
clock_format       = ""        # boş/yazılmamış: [locale] hour_cycle (format_hm); yazılırsa strftime kalıbı
date_format        = "%A, %e %B"  # gün/ay adları LC_TIME yerel ayarıyla (nuxut::locale)
fingerprint        = true    # fprintd varsa
fingerprint_tries  = 3       # 1..=10
show_media         = false
show_notifications = false
```

### Tetikleme

**Durum: shell tetikleyicisi AÇIK** (2026-10-16, karar: kullanıcının Kilitle düğmesi
hiçbir şey yapmıyordu, çünkü logind `Lock` sinyalini dinleyen yoktu; kök neden ve tasarım
`docs/design/lock-trigger.md`). Önceki durum KAPALI'ydı (karar); açık kalan iki
madde (gerçek fprintd cihazı, gerçek logind'le `LockedHint` sırası) elle, kullanıcıyla
doğrulanacak. Etkinleştirmeden önce her madde
kanıtıyla (commit/test/betik) işaretlenir. Canlı betikler `lock/tools/live/` altında (kullanım
`common.sh` başında: fake-auth debug derlemesi, `NUXUT_TEST_NAME=test-lock` izole sway,
`flock -o`; çalıştırıcı `run.sh`). 01–08 izole başsız sway'de (her betik taze compositor,
`start --owner`) iki ardışık tam koşuda geçti; framework main `73a6cd9` (2026-10-11).
Betik notu: sway klavye odağını kilit yüzeyine ilk karesinden sonra verir; o ana dek gelen
sanal tuşlar hiçbir yere gitmez (sızma değil). Betikler bu yüzden ilk kareyi bekler ve açmadan
önce alanı Escape ile temizler (`common.sh` `wait_first_frame`, `unlock`).
- [x] `nuxut-wayland` keymap yeniden oynatma güvenlik düzeltmesi (`747a935` → framework main'de
      `952e960`); `nuxut-lock` framework main `e08b3e6` ile derlenip test edildi
      (`LockError::InDraw` dahil).
- [x] Temel akış: yanlış şifre açmaz, doğru şifre 0 ile çıkar — `01-unlock.sh`.
- [x] Odak değişirken yazılan tuşlar kilit dışına gitmiyor: kendi `kitty --class nuxut-test`
      tuzak pencerem (`capture.sh`, ham kip) + `wtype`; kilit kurulurken ve kilitliyken
      yazılanların hiçbiri o pencereye ulaşmıyor — `02-focus-keys.sh` (tuzak yalnızca
      `ONCE|x` aldı: kilitten önceki tek `x`; sonrası 0 bayt).
- [x] `kill -9` → oturum kilitli kalıyor, ikinci örnek devralıp açıyor (G9) — `03-kill9.sh`
      (ölümden sonra tuzak 0 bayt; sway'in opak "terk edilmiş kilit" dolgusu).
- [x] 2 monitör ve kilitliyken çıkış ekle (ölçekli)/çıkar (G5) — `04-outputs.sh`.
- [x] Caps Lock uyarısı (F2): `test-compositor.sh keylock caps on`'dan hemen sonra (araya
      `wtype` girmeden; her `wtype` modifier'ları sıfırlar) uyarı görünür — `05-capslock.sh`.
- [x] `--ready-fd` (G7): "locked" yalnızca `locked` + tüm kilit yüzeylerinin ilk karesinden
      sonra (sunum kapısı, aşağıda); ikinci örnek yalnızca birincil "locked" iken hemen yazar;
      geçersiz fd kilidi engellemez — `06-ready-fd.sh`. Gerçek askıya alma/devam izole
      compositor'da yapılamaz: elle, kullanıcı onayıyla (açık).
- [x] İkincil örnek fail-open değil (`instance.rs`): birincil `pending` → kilitlenene dek
      bekler; `done` → birincil çıkınca kilidi kendisi kurar; 8 sn'de kilit yoksa
      `EXIT_NOT_LOCKED` — `08-secondary.sh` (fake-auth test kancası
      `NUXUT_LOCK_TEST_HOLD_{PENDING,DONE}_MS`, yalnızca sahte doğrulayıcı etkinken; kapalı
      ikilide adı bile yok: `lock/tests/no_fake_auth.rs`).
- [x] Duvar kağıdı bulanıklığı (F4) — `07-blur.sh` (sol üst kenar yoğunluğu keskin 0.023 →
      bulanık 0.004). Köşede kalan yumuşak leke dama tahtası deseninin ayna kenar koşulundaki
      doğal sonucu (framework `fddaeca` notu), keskin clamp lekesi yok.
- [ ] fprintd var / yok (yoksa yalnızca şifre, hata yok): özel bus testleri
      (`lock/tests/fingerprint_dbus.rs`); canlıda sahte kip sistem bus'ını bilinçli kapatır,
      gerçek cihazla elle. Test kararlılığı (2026-10-14): tam workspace koşusunda
      yük altında düşmelerin kök nedeni `nuxut-build.slice` bellek kısması (MemoryHigh) —
      testin kendi `dbus-daemon`'u/sahte fprintd'si saniyelerce uyuyor; testler ürün süre
      sınırlarını (çağrı 2 sn, olay 5 sn, PAM 5 sn) kullanıyordu ve sahte fprintd "yanıttan
      sonra" sinyallerini 60 ms sabit gecikmeyle yayınlıyordu (gecikmeli yanıtın gerisinde
      kalan sinyal haklı olarak "eski nesil" sayılıp atılıyordu). Artık sinyaller istemcinin
      `Scanning`'i (yanıtı aldığının kanıtı) ile tetiklenir, beklemeler sonuca dayalı (120 sn
      yalnızca asılı kalmaya karşı), reddetme iddiaları bir sonraki olumlu olayla senkron.
      Ürün kodunda yarış bulunmadı: yükte zaman aşımları `Unavailable`'a düşer (fail-closed).
- [ ] `LockedHint` sırası: `true` yalnızca sunum kapısından sonra, `false` yalnızca `unlock_session`
      `Ok` olunca (kod `37524bb`; özel bus testi `lock/tests/logind_dbus.rs` çağrıyı doğrular,
      uygulama sırası canlıda gerçek logind'le — kullanıcının oturumuna dokunacağı için elle).

- **logind `Session.Lock` sinyali** (shell `services/idle/logind.rs` dinler, boşta ayarlarından
  bağımsız; yalnızca login1'in benzersiz sahibinden ve kendi oturum yolumuzdan; `Unlock`
  iletilmez): `nuxut-lock` başlatılır (`services/lock_launcher.rs`; sabit yol: `current_exe`
  kardeşi, argv sabit, kabuk yok; çocuk reap thread'i; kendi çocuğu çalışırken ikincisi
  başlatılmaz). `shell.toml [lock] listen_logind = false` dinlemeyi kapatır (kendi
  kilitleyicisini kullanan için; varsayılan `true`).
  Sinyal beklenirken 1,5 sn (`LOCK_SIGNAL_GRACE`) içinde gelmezse ve oturum kilitlenmediyse yine
  doğrudan başlatılır. Başlatılamazsa ya da `nuxut-lock` 0 dışı çıkarsa **bildirim** açılır (sessiz
  başarısızlık yok). Oturum nesnesi: kendi PID → compositor PID (`hyprland.lock`; kabuk systemd
  servisiyken) → `session/auto` (`docs/design/lock-trigger.md` "İkinci tur").
- **`nuxut-shell msg lock`** ve oturum kartı "Kilitle" → logind `Session.Lock()` çağrısı →
  yukarıdaki sinyal. Tek yol: hypridle ve `loginctl lock-session` ile aynı davranış; SUPER+L temel
  katmanda `<kabuk> msg lock || loginctl lock-session` (ile anlaşma: kabuk çalışıyorsa
  fail-safe dahil onun yolu, çalışmıyorsa logind'e doğrudan). Fail-safe: logind'e ulaşılamazsa, çağrı başarısızsa ya da
  sinyali dinleyen bağlantı hazır değilse `nuxut-lock` doğrudan başlatılır (kart hatayı gösterir).
  Test örneği (`NUXUT_SHELL_INSTANCE`) gerçek logind'e kilit/çıkış göndermez.
  Kendi `nuxut-lock`'umuz compositor tarafından reddedilirse (başka kilit ekranı etkin) bildirim
  Ayarlar yolunu birebir söyler ("Ayarlar › Güç › Kilit › Nuxut kilit ekranını kullan"; metin–etiket
  eşleşmesi `tests/it/lock_notice_labels.rs`) ve "Ayarları aç" düğmesi `msg settings open power`
  çalıştırır (Ayarlar'ı o sayfada ayrık açar; test örneği açmaz).
- **Uyku öncesi** (`[idle] lock_before_sleep`, varsayılan AÇIK; karar 2026-10-16):
  shell logind `Inhibit("sleep", …, "delay")` tutar. `PrepareForSleep(true)` → kilitli değilse
  `Session.Lock()` → `Lock` sinyali → `nuxut-lock`; `LockedHint = true` gelince (sunum kapısından
  sonra; `--ready-fd` ile aynı an) ya da `InhibitDelayMaxUSec` dolunca (uyarı) inhibitor fd'si
  kapatılır (uyku asla süresiz engellenmez). Zaten kilitliyse hemen bırakılır.
  `PrepareForSleep(false)` → istenen kilit kurulmamışsa yeniden istenir (uyanınca masaüstü açık
  kalmaz), inhibitor yeniden alınır. Ayrıntı: `docs/design/lock-trigger.md`.
- **Sunum kapısı (karar):** "kilit kuruldu" (ready-fd, `LockedHint=true`, tek örnek
  `status` = `locked`) yalnızca compositor `locked` VE o anki tüm kilit yüzeylerinin ilk karesi
  sunulduktan sonra, bir kez (`state.rs` `NotifyLocked`, `PresentTracker`). `PRESENT_TIMEOUT`
  (2 sn) dolarsa bildirilmez, hata loglanır; oturum kilitli kalır (çıkmak kilidi bırakamaz,
  yalnızca terk edilmiş kilit bırakırdı), kare sonradan gelirse bildirim o zaman. "Sunuldu" =
  framework `App::surface_presented` (ilk tamponlu commit'e bağlı frame callback `done`).
  Kilitliyken eklenen çıkışın yeni yüzeyi onaydan önceyse bildirimi bekletir, sonra ikinci
  bildirim üretmez; kaldırılan çıkışın yüzeyi (`output_removed`) izleyiciden ve görünümlerden
  düşer, ev sahibiyse giriş kalan bir yüzeye taşınır.
- **LockedHint:** nuxut-lock kilit kurulunca (sunum kapısı) `SetLockedHint(true)`, açılışta
  `false`. Shell bunu izleyip
  kilitliyken açık paneli kapatır, popup'ları bastırır, animasyonları durdurur.
- **hypridle ile** (shell çalışmıyorsa da çalışır):
  ```ini
  general {
      lock_cmd = nuxut-lock                  # tek örnek: çift çağrı zararsız
      before_sleep_cmd = loginctl lock-session
      after_sleep_cmd = hyprctl dispatch dpms on
  }
  ```
  Shell de `Lock` sinyalini dinlediğinde ikisi birden başlatır; ikincisi tek-örnek kilidinden
  hemen çıkar. Uyku inhibitor'ını shell **ya da** hypridle tutar (`lock.toml`'da değil,
  `shell.toml [lock] sleep_inhibitor = true`; hypridle kullanan kapatabilir).
- **Kurtarma** (belge): Hyprland `misc:allow_session_lock_restore = true` önerilir; kilit çökerse
  TTY'den `hyprctl --instance 0 dispatch exec nuxut-lock` yeni kilidi alır.

### Framework talepleri (kabuk → framework)

- **F1 — ext-session-lock-v1 runtime yeteneği (nuxut-wayland).** sctk 0.21'in
  `session_lock` modülü kullanılabilir (drop'ta `destroy` = fail-secure). Öneri:
  ```rust
  impl<A: App> Context<A> {
      /// Protokol yoksa Err(Unsupported); zaten etkinse Err(AlreadyActive).
      pub fn lock_session(&mut self) -> Result<(), SessionLockError>;
      /// Pending/Locked iken; çıkış başına bir yüzey (ikincisi Err, protokol hatası değil).
      pub fn create_lock_surface(&mut self, output: OutputId) -> Result<SurfaceId, SessionLockError>;
      /// Yalnızca Locked iken unlock_and_destroy + flush; aksi Err (asla destroy'a düşmez).
      pub fn unlock_session(&mut self) -> Result<(), SessionLockError>;
      pub fn session_lock_state(&self) -> SessionLockState; // Idle | Pending | Locked | Finished
  }
  // App: fn session_locked(&mut self, cx) {}   fn session_lock_finished(&mut self, cx) {}
  ```
  Kilit yüzeyleri diğer yüzeylerle aynı yoldan: `surface_resized`, `draw`, `pointer`,
  `keyboard`, `keyboard_focus`, `ime`, imleç. Configure'dan önce tampon eklenmez; configure
  boyutu kullanılır, `ack_configure` her seferinde. **Değişmezler:** runtime hiçbir koşulda
  (`quit`, SIGTERM, `Drop`, hata) kendiliğinden `unlock_and_destroy` göndermez; `Locked`'ta
  `quit` → `destroy` (protokol hatası → compositor kilitli tutar) kabul edilen davranış.
  `zwlr_layer_shell_v1` yalnızca kilit kullanan uygulama için zorunlu olmamalı (yoksa
  `MissingProtocol` değil, `create_layer_surface` hata loglar) — öncelik düşük.
- **F2 — Caps Lock durumu (`nuxut-wayland` + nuxut-core).** `Modifiers`'a `caps_lock: bool`
  (ve `num_lock`), ve tuş olmadan değişimi bildiren `App::modifiers_changed(cx, surface,
  Modifiers)` (sctk `update_modifiers`'tan; Caps Lock'a basıp bırakınca uyarı hemen gelsin).
- **F3 — TextInput güvenli gizli kip (nuxut-ui).** `password(true)` iken metin
  `zeroize` edilen, kapasitesi `max_bytes`'a önceden ayrılmış (büyürken yeniden tahsis yok)
  tampon; `clear()` ve `Drop` sıfırlar; `take_secret(&mut self) -> Zeroizing<Vec<u8>>` (ya da
  çağıranın tamponuna yazan `write_secret_into(&mut [u8]) -> usize`) içeriği verip alanı
  sıfırlar. Preedit gizli kipte maskeli. `zeroize` nuxut-ui'ye bağımlılık olur (saf Rust, küçük).
- **F4 — Duvar kağıdı bulanıklığı (nuxut-image).** `Wallpaper`'a ya da `DecodeOptions`'a
  `blur: Option<f32>` (mantıksal px): çözerken 1/8'e küçült + 3 geçişli kutu bulanıklığı (arka
  plan thread'i, GPU'da ek iş yok; doğrusal filtreyle büyütülür).
  **Durum: bağlandı.** Framework `47cd684`: `Wallpaper::set_blur(Option<f32>)` (GPU dual
  Kawase, katman başına bir kez üretilip saklanır; bulanıkken animasyon donar). Kilit
  `OutputView::new`'da `set_source`'tan önce çağırır; karartma yedeği kaldırıldı (`dim` ayrı ayar).
- **F5 — headless test compositor'ı (çalışıyor):** kilidin canlı testleri yalnızca
  `tools/test-compositor.sh` (yalıtılmış headless sway) içinde yapılır; kullanıcının Hyprland'ı
  **asla** kilitlenmez.

### Güvenlik denetim listesi (her madde bir test)

| # | Özellik | Test |
|---|---|---|
| G1 | Şifre sıfırlanıyor | `SecretBuf` drop/clear sonrası tampon sıfır (ham işaretçiyle değil, `zeroize`'ın kendi testleri + kapasite/yeniden tahsis yok testi); TextInput gizli kip (F3) testleri; `Debug` çıktısında şifre yok |
| G2 | PAM hata yollarında kilit açılmıyor | `Authenticator` sahtesiyle her `AuthOutcome` ve thread panik/kanal kopması → `Unlocking`'e geçilmez; PAM kodu eşlemesi tablosu (yalnızca `PAM_SUCCESS`); `pam_start_confdir` ile `pam_deny`, eksik modül, conversation hatası, ikinci `ECHO_OFF` |
| G3 | Doğru şifre açıyor | `pam_exec expose_authtok` test servisiyle doğru/yanlış şifre (gerçek kullanıcı şifresi yok) |
| G4 | Kilit yüzeyi yaratılamazsa başarı yok | Durum makinesi birim testi (`finished` önce → çık(1), ready-fd yazılmaz); headless sway'de ikinci kilit örneği (`finished`) |
| G5 | Çıkış takıp çıkarma | Headless sway'de kilitliyken `create_output`/`output unplug` → yeni çıkışa yüzey, yüzey sayısı = çıkış sayısı |
| G6 | Klavye düzeni değişimi | Headless sway'de kilitliyken `input type:keyboard xkb_layout tr` ↔ `us`; sahte doğrulayıcıya giden baytlar beklenen UTF-8 |
| G7 | Ekran kapat/aç | `output * power off/on` sonrası kare akışı sürüyor, açma animasyonu timer yedeğiyle tamamlanıyor |
| G8 | Kilitliyken başka yüzeyler girdi almıyor | Headless sway'de bir layer-shell test istemcisi + kilit; `wtype` ile girdi → test istemcisi olay almıyor, kilit alıyor |
| G9 | Sinyal / çökme | `Locked`'ta SIGTERM yok sayılır; SIGKILL sonrası compositor kilitli (sway: kilit yüzeyi yok → kırmızı/boş, oturum açılmıyor) |
| G10 | Artan gecikme ve faillock mesajı | Saf durum testi: 3. hatadan sonra gecikme dizisi, gecikmedeyken gönderim reddi; PAM `ERROR_MSG` arayüze sınırlı ve temiz ulaşır |
| G11 | Tek örnek | İkinci `nuxut-lock` 0 ile çıkar, ikinci kilit nesnesi yaratmaz |
| G12 | Log temizliği | Test log yakalayıcısı: hiçbir log satırında test şifresi yok |
| G13 | Kullanıcı görseli kilit sürecinde çözülmez | `images.rs`: avatar sahte yalıtılmış çözücüden geçer (tek `decode_file`, `file_key("avatar")`, `Untrusted`, kırpılmış 256²), işçi hataları (`Unavailable`/`Protocol`/`Timeout`/`Killed`) → baş harfler, işçisiz `Sandbox`ta SVG `Unavailable`; görünümler tek `WallpaperDecoder`; headless sway (`NUXUT_LOCK_INSTANCE=test-wp`) bulanık JPEG duvar kağıdı + açma |

### Kod düzeni (öneri)

| Yol | İçerik |
|---|---|
| `crates/nuxut-pam/` | FFI: `authenticate(service, user, &SecretBuf) -> AuthOutcome`, conversation, `current_user()`, `SecretBuf` |
| `crates/nuxut-wallpapers/` | Yerleşik duvar kağıtları (shell + kilit + ayarlar): gömülü SVG'ler (`assets/`), `parse_source`, `render`/`render_preview`, `deband`, `cache` (önbellek PNG'si), `Renderer`/`Slot`, kendi Fluent kataloğu (`locales/`, `wallpapers-…`) |
| `lock/src/main.rs` | CLI, tek örnek, `PR_SET_DUMPABLE`, `nuxut::run_with` |
| `lock/src/state.rs` | Saf durum makinesi (Pending/Locked/Unlocking, gecikme, deneme) — testlerin çoğu burada |
| `lock/src/auth.rs` | `Authenticator` trait, PAM thread'i, fprintd istemcisi |
| `lock/src/ui.rs` | Çıkış başına görünüm (saat, avatar, alan, durum) |
| `lock/src/config.rs` | `lock.toml` şeması ve doğrulama |
| `lock/src/images.rs` | Duvar kağıdı + avatarın yalıtılmış çözücüleri (`LockImages`) |
| `src/services/lock_trigger.rs` (shell) | logind `Lock`/`PrepareForSleep`/inhibitor, çocuk süreç başlatma, `msg lock` |

## polkit ajanı (`nuxut-polkit`)

Güvenlik açısından kritik. Tasarım, tehdit modeli, doğrulanan polkit 127
yardımcı protokolü ve tasarımdan sapmalar: [`docs/design/polkit.md`](design/polkit.md).

- **Konum:** çalışma alanı üyesi `polkit/`, ikili `nuxut-polkit`, `unsafe_code = "forbid"`
  (parola tamponu, `PR_SET_DUMPABLE`, `getpwuid_r` → `crates/nuxut-pam`; `kill`/pidfd/
  `PR_SET_PDEATHSIG` → `rustix`).
- **İki süreç:** `nuxut-polkit` yerleşik ajandır (yalnızca zbus, Wayland yok): sistem bus'ında
  `/org/nuxut/PolicyKit1/AuthenticationAgent`, kendi logind oturumuna `fallback = false` ile
  kaydolur, istekleri sıraya koyar (aynı anda tek diyalog). Her istek için
  `nuxut-polkit dialog` başlatılır: stdin'den tek satır JSON (parola alanı yok), stdout'a tek
  satır sonuç. Parola, yardımcı bağlantısı ve arayüz yalnızca diyalog sürecindedir.
- **Kurallar:** `BeginAuthentication`/`CancelAuthentication` göndereni o anki
  `org.freedesktop.PolicyKit1` sahibi değilse `…Error.Failed`; iptal/Esc/zaman aşımı
  `…Error.Cancelled`; diyalog çökerse/öldürülürse `…Error.Failed` (yetki polkitd'de, yardımcı
  `SUCCESS` yazmadıkça yok). polkitd giderse istekler iptal; yeniden kayıt yalnızca kaydı kabul
  edenden farklı, boş olmayan yeni sahipte (etkinleştirme yarışında "already exists" döngüsü yok); başka polkit ajanı
  kayıtlıysa uyarı + geri çekilme (30 sn, 2 dk, 5 dk) — hyprpolkitagent'a dokunulmaz.
- **Arayüz:** framework layer-shell `Overlay`, `KeyboardMode::Exclusive`, `nuxut::ui::Dialog`
  (karartma + kart), `TextInput` parola kipi, birden çok kimlikte `Select`; renk/hareket tema
  token'larından, metinler `polkit/locales` (`polkit-…`, tr + en taslak). İlk sunumdan sonra
  400 ms tuş/IME yok sayılır (yazı-önü kapısı). İsteyen: doğrulanmış sistem yolu (root sahipli,
  yalnız root yazabilir, kullanıcı yazılabilir kök dışı, düz ASCII) etiketsiz; değilse
  "doğrulanmamış konum" (ev `~` ile) ya da `comm` "doğrulanmamış ad". Yol/ad/giriş adı/eylem
  kimliği kaçışlı (`\u{…}`) ve ortadan kırpılır.
- **Yapılandırma:** `shell.toml [polkit]` `enabled` (true), `timeout_s` (300; 30..=3600) —
  polkit ajanı salt okunur okur.
- **Kod:** `agent/` (polkit D-Bus, kayıt, `queue.rs` saf kuyruk, `process.rs` diyalog süreci),
  `dialog.rs` (arayüz), `session.rs` (diyaloğun saf durum makinesi), `protocol.rs` + `helper.rs`
  (yardımcı), `request.rs` (sözleşme), `text.rs` (gösterilen dış metin temizliği), `logind.rs`.
- **Test:** birim (protokol ayrıştırıcı + düşmanca satırlar, kuyruk, oturum makinesi, gönderen
  denetimi, kimlik seçimi, metin temizliği); `tests/agent_dbus.rs` (polkit) özel `dbus-daemon`'da sahte
  polkitd + logind ve sh diyaloğuyla; `tests/e2e_fake.rs` (`--features fake-polkit`, yalnızca
  debug) gerçek polkit ajanı + başsız diyalog + sahte yardımcı. Canlı arayüz yalnızca izole sway'de,
  sahte yardımcıyla. Gerçek polkitd'ye parola gönderilmez, kullanıcının oturumuna kayıt yapılmaz.
  `dbus-daemon` yoksa D-Bus testleri başarısız olur; bilinçli atlama `NUXUT_POLKIT_SKIP_DBUS_TESTS=1`.

## Giriş ekranı (`nuxut-greeter`)

Karar: ADR-0010 (greetd + kendi greeter'ımız, Hyprland içinde; 2026-10-16). Tasarım,
akış, tehdit modeli, kurulum/geri alma ve deneme talimatı: `docs/design/greeter.md`. Kod `greeter/`
(çalışma alanı üyesi), kurulum dosyaları `dist/greeter/`.

- greetd (root) PAM'i yapar; greeter `greeter` kullanıcısıyla, greetd soketiyle (`GREETD_SOCK`,
  uzunluk + JSON, ≤ 64 KiB) konuşur; tek istek uçuşta, kopan bağlantı Enter ile yeniden kurulur.
- Kilitle ortak: görünüm (`nuxut_lock::view::OutputView`), durum satırları, artan gecikme,
  yalıtılmış avatar/duvar kağıdı çözümü, `lock.toml` (`LockConfig::load`, artık kilit
  kütüphanesinde), `SecretBuf`, `PR_SET_DUMPABLE=0`.
- Tema kullanıcının evinden değil, sisteme yayımlanan kopyadan (`/var/lib/nuxut-greeter/config`,
  `XDG_CONFIG_HOME` ve `XDG_STATE_HOME` olarak); son seçim `/var/lib/nuxut-greeter/state/state.toml`.
- Klavye düzeni sistem ayarından (`localectl`), sağ altta gösterilir; birden çok düzende geçiş
  (`hyprctl switchxkblayout`). Ekran okuyucu: `a11y` özelliği + yayımlanan `greeter.toml`.
- Sistem değişikliği yalnız kullanıcı onayıyla, kök yardımcının `greeter` alanıyla (ADR-0008;
  `nuxut-shell login-screen apply|revert`, pkexec, polkit `org.nuxut.system-helper.greeter`):
  Hoş geldin "Sistem görünümünü Nuxut'a geçir" ya da `install.sh --system-theme`. Önce kayıt
  (önceki DM, greetd ayarı yedeği), hata = geri dönüş, `revert` geri alır; görünüm yalnız çağıranın
  sahip olduğu düz dosyalardan yayımlanır. SDDM kullanılmaz.

| Dosya | İçerik |
|---|---|
| `greeter/src/protocol.rs` | greetd iletileri, çerçeve, sınırlar, şifre yanıtının sıfırlanan tampona kodlanması |
| `greeter/src/client.rs` | Soket thread'i, yeniden bağlanma |
| `greeter/src/flow.rs` | Saf durum makinesi (istek-yanıt sırası, kullanıcı değişimi, kimlik hatası → yeni oturum, gecikme) |
| `greeter/src/{sessions,users,remember,power}.rs` | `.desktop` oturumları, `/etc/passwd`, son seçim, logind güç |
| `greeter/src/{view,app}.rs` | Form ve Wayland uygulaması (çıkış başına katman yüzeyi) |
| `greeter/src/keyboard.rs` | Klavye düzeni listesi, doğrulama, compositor'a geçiş |
| `crates/nuxut-system-helper/src/greeter.rs` | Kök yardımcının `greeter` alanı: apply/revert/status, görünüm yayımı |
| `src/services/login_screen.rs` | `nuxut-shell login-screen` (kullanıcı tarafı: durum, Hyprland düzeni, pkexec) |
| `greeter/tests/` | Sahte greetd ile uçtan uca akış, i18n kataloğu |
| `greeter/tools/live/run.sh` | İzole sway + sahte greetd canlı testi |

## Maskot (Fiyu)

Framework `nuxut::ui::Mascot` (2026-10-13, kullanıcı; yerleşim ve gerekçe
`docs/design/mascot-placement.md`, genel kural framework `docs/design/mascot.md`). Shell'de
yalnızca boş/sonuç yok/hata durumlarında, 96 px, alanda ortalı (framework `nuxut::ui::EmptyState`):
bildirim merkezi boş → `Empty`; başlatıcıda sonuç yok → `NoResults`; pano geçmişi boş/kapalı →
`Empty`; pano izlenemiyor → `Error`. ASLA bar, kilit ekranı, polkit/PAM, uygulama ikonu
(bekçi `tests/it/mascot_placement.rs`). `[appearance] mascot = false` ya da yer yetmezse
maskot yer kaplamaz, metin ortada, boşluk kalmaz; gizlemede bildirim merkezi eski sessiz ikonuna
döner (`EmptyState::with_fallback_icon`, `NotificationsOff`; 2026-10-14), başlatıcı/pano
ikonsuz kalır. "Güncelleme bitti" (`Celebrate`): sürüm
artışında iç bildirim kartında canlı maskot (`InternalNotice::with_mascot`; bkz. "Güncelleme bitti bildirimi").

## Ekran okuyucu (AT-SPI; framework ADR-0011, t127)

Karar 2026-10-15/16; kabuk tarafı 2026-10-17 (`docs/design/screen-reader.md`). `nuxut`
bağımlılığında `a11y` özelliği açık; at-spi2-core yoksa adaptör kurulmaz, Orca yokken ağaç hiç
kurulmaz (boşta 0 CPU).

- **Ağaç (ana yüzey, monitör başına):** kök "Nuxut masaüstü".
  - `Toolbar` "Bar": her bar widget'ı `BarWidget::access_items` bölgeleri kadar düğme. Varsayılan
    tek düğme, ad = widget tür adı, **değer = widget'ın kendi ipucu** (Uyanık tut, gizlilik,
    sistem izleyici, büyüteç hapı, klavye düzeni, sıradaki etkinlik…). Saat: görünen saat + tam
    tarih; etkin pencere: başlık; durum: görünen göstergelerin ipuçları ("Wi-Fi: …, Ses %64,
    Pil %80"); çalışma alanları: yuva başına "Çalışma alanı N" (etkin basılı) ve sayfa okları.
  - Açık kenar paneli `Pane` "Panel" → kart başına `Group` (ad kart türü; `widgets::access_card`)
    ve kartın içeriği (`PanelWidget::access`): oturum (kullanıcı, eylem düğmeleri; onay
    beklerken ad onay etiketi), hızlı anahtarlar (`Switch`), ses (çıkış/giriş kaydırıcısı + sessiz
    düğmesi, cihaz listeleri), karıştırıcı, parlaklık, gece ışığı, büyüteç, ekran klavyesi, güç
    (pil, profil radyo grubu, şarj sınırı), ağ (liste; şifre istemi `PasswordInput`, yalnız
    maske), Bluetooth (liste; eşleştirme sorusu canlı bölge), medya (başlık, düğmeler), takvim
    kartı, bildirim merkezi (Tümünü temizle, Rahatsız etme, görünen kartlar).
  - Saat popout'u `Pane` "Takvim ve saat": saat + bugünün tarihi, seçili gün grubu (canlı bölge;
    ok tuşlarıyla gün değişince etkinlikler okunur).
  - Açık başlatıcı `Dialog`: arama alanı (odak; seçili sonuç `active_descendant`), sonuç listesi
    (`ListBox`, satır anahtarı katalog/pano kimliği), boş durum, eylemler sayfası.
  - Açılır bildirim kartları `Group` "Bildirimler", kart başına `Live::Polite`; ad başlık +
    gövde önizlemesi (160 grafem) — Orca canlı bölgede adı okur.
  - OSD görünürken `Label` (canlı bölge): "Ses %64", "Mikrofon kapalı", "Büyüteç 2,0×".
  - Kod: `src/screen/access.rs`, `src/launcher_view/access.rs`, `src/app/a11y.rs`, kartların
    `access`/`access_action` yöntemleri.
- **Klavye gezinmesi (Orca'nın bar ve panelleri dolaşma yolu):** Hyprland'de Orca'nın kendi
  inceleme komutları çalışmadığından ve Orca yalnız odaktakini okuduğundan kabuk kendi klavye
  gezinmesini sunar (ekran okuyucusuz klavye kullanıcısı için de). Yüzey klavye odağındayken
  ve bileşen tuşu tüketmediyse: Tab / Shift+Tab ve oklar odağı **son kurulan ağacın** eylemli,
  etkin, görünen düğümleri arasında ağaç sırasıyla taşır (sona gelince başa), Enter / boşluk
  etkinleştirir (AT `Click` ile aynı yol), kaydırıcıda sol/sağ değeri adımlar. Odak ağaçta
  bildirilir (Orca okur) ve `primary` odak halkasıyla çizilir. Bar'a giriş: **`msg bar focus`**
  (kısayol `bar_focus`, varsayılan `CTRL+ALT+Tab`): klavye özel kipe geçer, odak bar'ın ilk
  öğesine; Escape ya da odak kaybı çıkar. Etkinleştirme bir yaprak açtıysa odak onun ilk öğesine
  geçer. Hedefler ağaçtan kurulur (ekran okuyucu çalışmıyorsa ağaç yalnız bu tuşlar için
  yerelde kurulur; adaptöre gitmez).
- **Eylemler ve güvenlik kuralı (2026-10-17, HIGH bulgu):** a11y bus'ı oturumdaki her sürece
  (Flatpak'te varsayılan izinli) açıktır; AT eylemi kullanıcı niyeti değildir. **AT yalnız okur,
  odak taşır ve gezinir.** AT `Click`'i kabuğun kendi işaretçi işleyicisine benzetilir (hareket →
  bas → bırak, işaretçi eski yerine) ama sonuç eylemi yalnız gezinmeyse uygulanır (yaprak
  aç/kapat/sabitle, çalışma alanı, takvim ayı: `app::a11y::assistive_allowed`); uygulama
  başlatma, pano öğesi, oturum eylemleri, bildirim eylemleri, Bluetooth/ağ onayları, ayar
  anahtarları atılır (log: yalnız "reddedildi"). Kaydırıcı değeri ve metin AT'den değişmez. AT
  `Focus` gezinme odağını taşır. Tam etkinleştirme **gerçek girdiyle**: işaretçi ya da kabuğun
  klavye gezinmesi (Wayland tuşu → `Source::Keyboard`: bileşenin kendi eylemi — kaydırıcı adımı,
  bildirim kartı — ya da düğüme tıklama). Orca kullanıcısı odağı taşır, onayı kendi klavyesi verir.
  Polkit, ekran paylaşım seçicisi ve kilit ekranı ağaç/eylem yayımlamaz; eklenirse bekçi
  (`tests/it/at_policy.rs`) `AT_POLICY:` belgesi ister.
- **Ekran klavyesi (`nuxut-osk` yüzeyi):** kök ve `Group` "Ekran klavyesi" → tuş başına `Button`
  (ad `osk_view::accessible_name`). **Tuşlar eylemsiz** (2026-10-16): AT'den sanal klavyeyle
  odaklı pencereye tuş gönderilemez (ADR-0011 karar 6). Anahtarlama/göz izleme erişimi
  gerekirse ayrı karar.
- **Dosya seçici (`nuxut-filechooser`, ayrı süreç):** kök pencere başlığı; `Toolbar` (Geri, Üst
  klasör, `Navigation` "Konum", Gizli dosyaları göster, Yeni klasör, arama), "Yerler ve
  aygıtlar", "Dosyalar" (satır anahtarı dosya adı), ad alanı, "Dosya türü", seçim kutuları,
  durum satırı (`Live::Polite`), Vazgeç/kabul; diyalog açıkken yalnız diyalog. **AT izin listesi**
  (2026-10-17, portal sandbox kaçışı bulgusu): AT `Click` yalnız gezinme/seçme düğümlerinde
  (araç çubuğu — yeni klasör hariç —, yerler, dosya satırları, filtre, seçim kutuları, Vazgeç);
  **kabul düğmesi, diyalog düğmeleri (üzerine yazma onayı) ve yeni klasör AT'ye eylemsiz**; onay
  yalnız gerçek Enter/işaretçi. Konum ve ad alanı AT için salt okunur. Düşmanca uçtan uca test:
  `examples/at_probe` (`docs/design/screen-reader.md` "Güvenlik"). Kod:
  `src/filechooser_view/access.rs`.
- **Güvenlik (ADR-0011 tehdit modeli):** logind `LockedHint` true iken
  `cx.set_a11y_suspended(true)` (`app/idle.rs`): ağaç içeriksiz, AT eylemleri ve odak atılır;
  kilit açılınca sürdürülür. Pano geçmişi öğe adı `nuxut::access::preview` (40 grafem); bildirim
  ad/açıklaması sınırlı. Parola alanları yalnız maske, AT metin yazamaz. `AccessItem` `Debug`'ı
  yalnız uzunluk yazar; ağaç içeriği loglanmaz.
- **Bilinen sınırlar:** Hyprland'de Orca'nın kendi tuş komutları (düz inceleme, "neredeyim")
  çalışmaz (`KeyboardMonitor` yok; ADR-0011) — gezinme kabuğun Tab/ok yolu ile. Takvim ızgarasının
  hücreleri tek tek ağaçta değil (seçili gün + etkinlikler). Tepsi öğeleri tek düğme ("Tepsi").
  **nuxut-lock** ağaç yayımlamaz (framework kilit yüzeylerine adaptör kurmaz; açmak framework
  izni + güvenlik incelemesi gerektirir, `docs/design/screen-reader.md` "Kilit ekranı").
  nuxut-polkit MVP dışı. Dosya seçici oturum kilidinde askıya almaz (açık iş, 2026-10-16).
- **Doğrulama:** birim (`screen::access::tests`: bar ağacı, gezinme; kart testleri) +
  `framework/tools/a11y-e2e.sh` içinde kabuk ve `atspi_dump` (bar, panel, açılır kart, odak,
  başlatıcı) ve **gerçek Orca** (konuşma sunucusu yok, `--debug-file`; `wtype` yalnız test
  compositor'ına): `docs/design/screen-reader.md` "Doğrulama".
## Sistem görünümü (Plymouth / GRUB / Limine)

Tasarım ve tehdit modeli: `docs/design/boot-theme.md`; kök yardımcı: ADR-0008.

- `nuxut-shell system-theme status|preview|apply|revert` (`services::system_theme`): kullanıcı
  tarafı yalnız algılar, planı (değişecek yollar dahil) gösterir, seçili paletin **koyu** kipinden
  renkleri ve önyükleyici ölçeğini (1–3; en büyük ekranın fiziksel yüksekliği / Hyprland ölçeği)
  hesaplar, sisteme dokunmadan PNG önizleme üretir. Uygulama ve geri alma
  `pkexec /usr/lib/nuxut/nuxut-system-helper boot-theme …` ile; çıktı satır başına JSON olay.
  Çıkış kodu 3 = yönetici onayı verilmedi.
- `crates/nuxut-boot-theme` (saf): Plymouth two-step teması, GRUB `theme.txt` + 9 parçalı PNG'ler
  (PF2'yi yardımcı `grub-mkfont` ile üretir), Limine renk bloğu (`limine.conf` başında, işaretli),
  `plymouthd.conf`/mkinitcpio/grub.cfg ayrıştırıcıları, algılama (`Probe`), plan, PF2 okuyucu, önizleme.
- `crates/nuxut-system-helper` (root, pkexec): beyaz liste (`policy`), doğrulanmış dizin
  tanıtıcıları (`safefs`), hedef başına işlem ve geri alma (`ops::Tx`), bildirim
  (`/var/lib/nuxut/system-theme/manifest.json`), dış komutlar (`cmd`). GRUB: damla dosyası
  `/etc/default/grub.d/90-nuxut-theme.cfg` (kullanıcının `/etc/default/grub`'ı değişmez) +
  `grub-mkconfig` → `grub-script-check` → giriş sayısı denetimi → atomik yerine koyma. Limine:
  kayıtlı sağlama toplamı (`limine enroll-config`) ya da birden çok aday varsa dokunulmaz.
  Plymouth: HOOKS'ta varsa initramfs yedekli `mkinitcpio -P`. systemd-boot temalanamaz (not).
- Hoş geldin adımı ("Sistem görünümünü Nuxut'a geçir", varsayılan seçili) Ayarlar'da; sözleşme
  tasarım belgesinde. Test dönemi: `install.sh --system-theme` / `--system-theme-revert`.

## Kod düzeni

| Dosya | İçerik |
|---|---|
| `src/config.rs` | `ShellConfig` şeması, yükleme, doğrulama, `frame_insets()` |
| `src/config_migrate.rs` | shell.toml sürüm göçü (ADR-0002): `config_version`, `MIGRATIONS`, atomik yazıcı arayüzü, geri al |
| `src/config_heal/` | ayarların kendini onarması: shell.toml kuralları (`RENAMES`, `RETIRED`), yedek, bildirim, geri al, koru listesi; framework.toml sözdizimi (`crates/nuxut-config-heal` saf çekirdek) |
| `src/schema.rs` | `nuxut-shell schema`: ayarlar uygulamasının okuduğu TOML şeması |
| `src/widgets/` | Widget kaydı (`registry.rs`), `BarWidget` trait'i, widget'lar |
| `src/sheet.rs` | Kenardan büyüyen blob yaprağı (popout, çekmece) |
| `src/calendar_view.rs` | Takvim görünümü: saat popout'u ve panel kartı (`widgets/calendar_card.rs`) kipleri, ızgara, etkinlikler, sorgu aralığı ve birleşimi |
| `src/launcher_view.rs` | Başlatıcı içeriği: SearchField + ResultList + eylemler alt sayfası (PageStack), ikon önbelleği, boş liste maskotu |
| `src/emoji_view.rs` | Başlatıcının emoji kipi: `GlyphGrid` (bölümler/düz küme), ad satırı, emoji font kapsaması süzmesi |
| `src/services/launcher/` | Başlatıcı servisi: keşif, inotify, arama, kullanım, hesap, emoji verisi/araması (`emoji.rs`, `data/`), başlatma |
| `src/services/default_apps/` | Varsayılan uygulamalar: mimeapps çözümü, adaylar, nuxut-apply yazımı, terminal listesi |
| `src/ipc.rs` | Komut satırı ve CLI ayrıştırma |
| `src/services/session.rs` | Kullanıcı bilgisi + logind eylemleri |
| `src/services/shell_state.rs` | Kabuk sürüm durumu (`$XDG_STATE_HOME/nuxut/shell-state.toml` `last_version`) ve "güncelleme bitti" kararı; gönderen `src/app/update_notice.rs` |
| `src/services/calendar.rs` | nuxut-calendard istemcisi |
| `src/services/tray/` | StatusNotifier watcher/host + DBusMenu |
| `src/services/notifications/` | Bildirim sunucusu + popup/merkez modeli |
| `src/layout.rs` | Saf geometri: `ShellLayout` (insets, `FrameGeometry`, bar şeridi/içeriği), `exclusive_zones`, `input_region`, `bar_slots`, `panel` (kapalı/açık dikdörtgen) |
| `src/model.rs` | `ShellModel`, workspace göstergeleri, tam ekran / odaklı pencere türetmeleri |
| `src/services/hyprland.rs` | IPC istemcisi + olay servisi |
| `src/services/clock.rs` | Saat biçimi doğrulama, biçimlendirme, hizalı tick, otomatik 12/24 saat (`hour_cycle`) |
| `src/services/timezone.rs` | timedate1 saat dilimi değişimi dinleyicisi |
| `crates/nuxut-dbus/` | Sistem servisi istemcilerinin ortak D-Bus altyapısı (eski `services/dbus_util`): `ServiceThread`/`ServiceContext`, `ObjectStore`, `NameOwner`, `Backoff`, `connect`/`close`/`tick_connection`, sınırlı değer/özellik okuyucuları, sinyal yardımcıları. UI'sız; kabuk servisleri doğrudan `nuxut_dbus::` kullanır |
| `crates/nuxut-nm/` | NetworkManager istemcisi (eski `services/network`): durum modeli, `NetworkService`, gizli bilgi ajanı. UI'sız: i18n yok, hata `NmError` türüyle döner; testleri `crates/nuxut-nm/tests/` (kabuğun `tests/support/`'unu `#[path]` ile bağlar). Kabukta `src/services/network.rs` yalnız yeniden dışa aktarır; metin `src/system.rs` `network_error_text`'te (`shell-network-error-*`; NM/BlueZ eşlemesinin tek yeri). `NmError` `Debug`'ı redakte |
| `crates/nuxut-bluez/` | BlueZ istemcisi (eski `services/bluetooth`): model, `BluetoothService`, eşleştirme ajanı. UI'sız; hata `BluezError`; testleri `crates/nuxut-bluez/tests/`. Kabukta `src/services/bluetooth.rs` yalnız yeniden dışa aktarır; metin `src/system.rs` `bluetooth_error_text`'te (`shell-bluetooth-error-*`). `BluezError` `Debug`'ı redakte |
| `crates/*` (sözleşme) | **Crate sözleşmesi:** shell/crates/* public API'sinde kırıcı değişiklik iniş ÖNCESİ settings'te derlenir (`cargo check --all-targets`, settings main'i dala karşı) ve settings düzeltmesi aynı dakikada iner; Ayarlar'la önceden koordine (62c67a6 settings main'i kırdı, 2026-10-14). Olay/hata enum'ları `#[non_exhaustive]`: yeni kol kırıcı sayılmaz (2026-10-14) |
| `src/app/mod.rs` | `nuxut::App` çekirdeği: output başına yüzeyler, exclusion eşleme, eylem/mesaj/timer yönlendirme (tek satırlık çağrılar) |
| `src/app/<özellik>.rs` | Özellik başına bağlantı (`impl Shell`): `start_<x>`, `on_<x>`, eylem/timer işleyicileri — hyprland, notify, tray, session, calendar, wallpaper, clock, input (odak/klavye), commands (IPC + config yeniden yükleme). Yeni özellik kendi dosyasında; işçiler yalnızca kendi modülünü düzenler |
| `src/screen.rs` | Output durumu: `Animated` insets/kavis, bar yerleşimi, yapraklar, isabet testi, hasar, çizim |
| `src/bar_damage.rs` | Bar hasarı: widget/uç başına kirli bayrak + son yerleşim; kare hasar dikdörtgenleri (birim testli) |
| `src/main.rs` | Giriş: CLI (`msg`), config yükle, `nuxut::run(Shell)` |
| `src/services/portal_env.rs` | Nuxut oturumunun portal ortamı (`XDG_CURRENT_DESKTOP=Nuxut:…`, saf + kayıt yoklaması) |
| `portal/` | `xdg-desktop-portal-nuxut` çalışma alanı üyesi: `appearance` (saf eşleme), `settings` (D-Bus arayüzü), `service` (bus + izleyici), `portals_conf` (`nuxut-portals.conf` türetme; `--write-portals-conf`); `tests/settings_dbus.rs` özel dbus-daemon |
| `dist/portal/` | `nuxut.portal`, D-Bus servis dosyası, systemd birimi (`nuxut-portals.conf` dosya değil, kurulumda türetilir) |
| `tests/it/` | Entegrasyon testleri tek ikilide (`cargo test --test it`): servis, i18n, kaynak bekçileri (`no_hardcoded_*`), `displays_ipc`. `mod` olarak bağlanır; ortak yardımcılar `tests/support/` (ikilide bir kez `#[path]` ile). Yeni entegrasyon testi `tests/it/<ad>.rs` + `tests/it/main.rs`te `mod <ad>;` |
| `tests/*.rs` | Süreç-global durum gerektiren ayrı ikililer: `notifications_memory` (RSS ölçümü), `notifications_dbus` (zamanlama + işçi sayımı), `tray_service` (işçi sayımı); her biri `tests/support/` ile `#[path]` bağlar |
| `tests/fixtures/` | Sahte soket/yapılandırma çıktıları: `hyprland/` canlı Hyprland 0.56.2 çıktıları (`src/` testleri de kullanır) |

## Kurulum (install.sh)

Derle → ikilileri atomik kur (çalışan örnek etkilenmez) → servis/masaüstü dosyaları → Hyprland
seçeneği → denetimler → çalışan kabuğu yeni sürümle yeniden başlat (`--no-restart`; ayrıntı
README). Kaldırma (`--uninstall`) ayarlara ve yedeklere dokunmaz.

**Kurulum dizini PATH'te** (kullanıcı kararı 2026-10-16: kabuk başlangıç dosyalarına
(.bash_profile/.profile/fish) yazılmaz, elle adım gerektiren çözüm yok). tty → bash → start-hyprland
yolunda `environment.d` okunmaz. Bu yüzden dizini Nuxut'un Hyprland katmanı koyar: `nuxut.conf`
oturumda bir kez `envd = PATH,<dizin>:$PATH` (hyprlang `$PATH`'i o anki ortamdan açar; Hyprland
`env`'i her yeniden okumada yeniden kurduğu için blok `# hyprlang if !NUXUT_PATH_ADDED` korumalı,
işaret sonda — yoksa PATH her yeniden yüklemede büyürdü; `envd` systemd/D-Bus'a da aktarır),
`nuxut.lua` `os.getenv` ile yalnızca yoksa ekler; oturum kancası `import-environment PATH …`.
Hyprland'den açılan terminaller ve `nuxut-shell.service` dizini alır. Sistem dizininde
(`/usr/bin` …) satır yok. Kabuk ve Ayarlar birbirini önce kendi dizininde arar (PATH'e
güvenmez). install.sh dosya varlığını değil etkin PATH'i (bu terminal + systemd ortamı) denetler;
`--fix-path` (environment.d) yalnızca Hyprland dışı systemd/uwsm oturumları için. Kalıcı çözüm
paket (`/usr/bin`) + giriş yöneticisi.

**Kendini güncelleme** (2026-10-16): betik `$SRC/shell`'i git ile günceller
ama çalışan kopya eskisidir; bu yüzden betikteki bir değişiklik eskiden ancak ikinci
çalıştırmada etkili oluyordu. Artık kaynak güncellemesinden sonra `$SRC/shell/install.sh`
çalışan kopyadan farklıysa (`cksum`) betik özgün argümanlarla (tırnak/boşluk korunur) bir kez
kendini yeniden exec eder. Bekçi `NUXUT_INSTALL_REEXEC=1`: yeniden exec edilen kopya bir daha
exec etmez; değişken betiğin başında silinir, kurulumun başlattığı süreçlere sızmaz. Dal
`NUXUT_BRANCH` ortamıyla, kaynak dizini `--src`/`NUXUT_SRC` ile aynen geçer. Paket adımı
yeniden çalışır (`--needed`; yeni sürüm yeni paket isteyebilir). Taze kurulum/yedek derlemeden
sonra olduğu için bir kez yapılır. Dosyasız çalışmada (`curl … | sh`) karşılaştırma yapılmaz.
Test: `tests/it/install_reexec.rs` (`tests/fixtures/install_reexec.sh`: geçici HOME, yerel bare
"origin", sahte cargo; tek yeniden exec, argümanlar, güncelken exec yok, bekçi).

**Depo adresi** (2026-10-16, herkese açık yayın): varsayılan `https://github.com/nuxut/<dizin>`
(HTTPS erişilemezse `ssh://git@github.com/nuxut`); betikte başka adres yoktur. Ayna:
`NUXUT_GIT_BASE` ve depo adları önekliyse `NUXUT_REPO_PREFIX` (`<önek>shell`; yalnız
`[a-z0-9-]`); var olan klonun origin'i bu ikisinden türeyen adreslerden biri olmalıdır. Ortamla
verilen ayna kaynaklar çekilince `$XDG_STATE_HOME/nuxut/install.conf`'a (0600, yalnız `base=` ve
`prefix=`) yazılır; sonraki çalıştırmada ortam yoksa oradan okunur (2026-10-16). Kayıt bize ait,
sembolik bağ olmayan ve grup/başkalarının yazamadığı bir dosya değilse betik durur. Kayıttan okunan
değerler ortamdakiyle aynı doğrulamadan geçer (güvenlik incelemesi 2026-10-17): taban yalnız
`https://`, `ssh://` ya da mutlak yerel yol, boşluk/denetim karakteri yok; önek `[a-z0-9-]`, `-` ile
başlamaz. Kayıt kullanılınca çıktıda "kayıtlı kaynak: …", ilk kayıtta "Kaynak aynası kaydedildi: …"
görünür. Özel adres yalnız kullanıcının durum dosyasındadır, betikte yoktur. Test:
`install_source_trust.sh` 16.

**Paket algılama** (güvenlik incelemesi 2026-10-16: doğrulanmamış AUR paketine güven yok): betik
AUR'dan hiçbir şey kurmaz, AUR yardımcısı (paru/yay) çağırmaz; bağımlılıklar yalnız resmî
depolardan `pacman` ile. AUR'daki `nuxut-git` kullanıcının seçimidir: kuruluysa (`pacman -Qi
nuxut-git` ya da `nuxut`) betik "paketle kurulu, paket yöneticinle güncelle" deyip kaynaktan
kurmadan çıkar. Kaynak yolunun üç şartı: sabit https/ssh uzak adres doğrulaması (aşağıda
"Kaynak güveni"), `--ref`/`NUXUT_REF` ile etikete ya da commit'e
sabitleme (aşağıda) ve `cargo build --locked`. Test: `tests/fixtures/install_source_trust.sh` 15
(paket algılama; AUR yardımcısı ve derleme yok) ve 12 (betikte AUR yardımcısı çağrısı yok). Betik kabuk başlangıç dosyalarına
(`~/.bashrc`, `~/.profile`…) hiçbir koşulda yazmaz.

**Kaynak güveni** (güvenlik incelemesi 2026-10-16; betik herkese açık `curl … | sh` giriş
noktası): `--src`/`NUXUT_SRC` ve `--prefix` mutlak yol olmalı (çalışma dizinine göreli yol yok);
kaynak dizini, önek ve bütün üst dizinleri bize ya da root'a ait ve grup/başkaları yazamaz —
yapışkan bitli paylaşımlı dizin (`/tmp`) de kabul edilmez (toctou-path-trust, 2026-10-16); kaynak dizini ve her klon (ve `.git`'i) bize ait, grup/başkaları
yazamaz, sembolik bağ değil — bu denetim herhangi bir git komutundan önce (klon yapılandırması
kanca/fsmonitor çalıştırabilir). Var olan klonun origin'i `$GIT_BASE`, HTTPS ya da SSH kanonik
adresinden biri değilse betik durur. `NUXUT_GIT_BASE` yalnız `https://`, `ssh://` ya da mutlak
yerel dizin (ayna/test). Yeniden exec yalnız doğrulanmış klonda izlenen, değişmemiş, sembolik bağ
olmayan ve HEAD'i tam seçilen commit (`origin/$BRANCH` ya da sabitleme) olan `install.sh`'ye; parmak izi yalnız `$0` bir
`install.sh` dosyasıysa alınır (borudan çalışmada `$0` "sh"tır, çalışma dizinindeki `sh`
dosyası sayılmaz). Yollar önce mutlak olmalı, sonra `realpath -m` ile kanonik biçime çevrilir ve
bütün türetilen yollar (kaynak, hedef dizini, `$BIN`, `$LIBEXEC`) bu biçimden gelir: güven
denetimi bağın değil gerçek dizinin üst zincirinde yapılır, denetimden sonra bağı değiştirmek
sonucu etkilemez (path-trust-bypass, 2026-10-16: kendi dizinimizdeki bir bağ, başkasının
yazabildiği dizindeki alt dizine gidiyorsa eski yürüyüş yalnız bağın tarafını denetliyordu).
Kurulum öneki de aynı zincir denetiminden geçer. Kalan TOCTOU yüzeyi yalnız kullanıcının kendi
dizinleri: yolun her bileşeni bizim ya da root'un, paylaşımlı yazılabilir hiçbir dizin yok; onları
değiştirebilen tek taraf aynı kullanıcı, o da zaten betiği değiştirebilir (tehdit modeli dışı). Tedarik zinciri (supply-chain, 2026-10-16):
her `cargo build` `--locked` (yalnız depodaki Cargo.lock'un sürüm ve sağlama toplamları; kilit
eskiyse kurulum durur — dört deponun main'deki Cargo.lock'u güncel tutulmalı); `NUXUT_BRANCH`
seçenek biçiminde olamaz (`git fetch origin --upload-pack=…`); `GIT_ALLOW_PROTOCOL=https:ssh:file`
(ext::, git://, http:// ve HTTPS'ten düşürme yok); AUR yardımcısı (paru/yay) hiç çağrılmaz:
Material Symbols resmî `ttf-material-symbols-variable`'dan (AUR `-git` kuruluysa dokunulmaz) —
kapsam doğrulandı (2026-10-16): framework ikon tablosunun 114 Material ligatürünün hepsi resmî 2.874
fontunda tek glife çözülüyor (harfrust ile şekillendirme; AUR `-git` ile aynı sonuç), yalnız
AUR'da olan isteğe bağlı imleçler için "AUR'da, kendiniz kurun" denir ve atlanır; Rust yalnız pacman
paketiyle (`rust`/`rustup`), hiçbir betik `curl | sh` ile indirilmez. Sabitleme: `--ref`/`NUXUT_REF`
(etiket ya da commit; depo başına `NUXUT_REF_SHELL` vb.), yoksa dal ucu; seçilen commit
çalışma ağacında doğrulanır, her deponun tam hash'i çıktıya ve `$XDG_STATE_HOME/nuxut/install-sources`
kaydına yazılır ("durum kaynak" → ikililer doğrulanınca "durum kuruldu"). Test:
`tests/it/install_source_trust.rs` (14 senaryo; geçici dizin paylaşımsız `CARGO_TARGET_TMPDIR`).

**Çakışan servisler** (2026-10-16, `docs/design/service-takeover.md`): dunst, mako, swaync, waybar,
quickshell, ags, eww artık yalnız uyarılmaz, devralınır (`dist/takeover/conflicting-services.sh`):
önce `$XDG_STATE_HOME/nuxut/takeover/services`'e kayıt, sonra systemd kullanıcı birimi devre dışı
(bildirim servisleri D-Bus etkinleştirmesine karşı maskelenir); bu oturumda durdurma yalnız Nuxut
kabuğu çalışacaksa. Hyprland `exec-once` satırına dokunulmaz, yeri yazdırılır. `--keep-services`
yalnız uyarır; `--dry-run` planı gösterir; `--uninstall` kayıttan geri getirir.

**Zorunlu ikililer** (2026-10-16: `nuxut-lock` ne derleniyor ne kuruluyordu, Kilitle hiçbir şey
açmıyordu): kurulum sonunda `$BINS`, `$LIBEXEC` ikilileri (portal, ekran paylaşım ve dosya
seçicisi) ve yardımcılar tek tek denetlenir, eksik varsa kurulum hatayla biter.
`tests/it/install_bins.rs` çalışma alanının özelliksiz her `[[bin]]`'inin kurulduğunu ve
paketinin `cargo build -p` listesinde olduğunu bekler. Kabuk açılışta kilit ikilisi yoksa WARN
ve bildirim ("Kilit ekranı kurulu değil") verir.

Masaüstü dosyaları dağıtıldıkları depodan kopyalanır, kabuk üretmez: Ayarlar'ın
`dist/nuxut-settings.desktop` ve gizli `dist/nuxut-welcome.desktop`'u (Hoş geldin penceresi
`nuxut-welcome` app-id'siyle Ayarlar ikonunu alsın) `~/.local/share/applications/` altına,
`Exec` mutlak `$BIN` yoluna çevrilerek; `dist/icons/hicolor/scalable/apps/nuxut-settings.svg`
`~/.local/share/icons/hicolor/…` altına. Kullanıcı hicolor dizinine `icon-theme.cache` yazılmaz
(bayat önbellek sonradan eklenen ikonları gizler). Takvimin webcal girdisi aynı kalıpla. Kaldırma
hepsini siler.

Portal arka ucu (`dist/portal/`): ikili `$PREFIX/lib/nuxut/xdg-desktop-portal-nuxut`,
`nuxut.portal` → `$XDG_DATA_HOME/xdg-desktop-portal/portals/`, D-Bus servis dosyası ve systemd
birimi (`Exec`/`ExecStart` mutlak yola; birim etkinleştirilmez). `nuxut-portals.conf` kopyalanmaz:
kurulan ikili `--write-portals-conf` ile kullanıcının Hyprland portal seçiminden türetip
`$XDG_CONFIG_HOME/xdg-desktop-portal/`'a atomik yazar (yalnızca yoksa ya da ilk satır `# nuxut-managed: portals`
önekiyle başlıyorsa; kullanıcı ilk satırı silerek sahiplenir, kaldırma da o zaman dokunmaz). `--dry-run`
kurulu ikili bayrağı tanıyorsa üretilecek içeriği gösterir. `hyprland-portals.conf` sonradan
değişirse aynı komut (ya da yeniden kurulum) dosyayı tazeler. Paketler: `xdg-desktop-portal`,
`xdg-desktop-portal-gtk`. Dosya seçici: `nuxut-filechooser` portalın yanına (`$LIBEXEC`) kurulur;
türetilen dosyada `org.freedesktop.impl.portal.FileChooser=nuxut;<önceki seçim ya da default>`
(Nuxut varsayılan, 2026-10-16; açık seçim, ör. `kde`, yedek arka uç olur ve `[X-Nuxut-Backup]`'ta
saklanır; `--no-filechooser` tek komutla geri alır, tercih dosyada kalır). Nuxut'un önceki
dosyası değişecekse `nuxut-portals.conf.nuxut-yedek-<zaman>` yedeği alınır (docs/design/file-chooser.md).

Ekran paylaşım seçicisi: ikili `$PREFIX/lib/nuxut/nuxut-sharepicker`; `dist/xdph/xdph-picker.sh install`
`$XDG_CONFIG_HOME/hypr/xdph.conf`'a işaretli `screencopy { custom_picker_binary = … }` bloğu yazar
(içerik değişirse önce `xdph.conf.nuxut-yedek-<zaman>`; kullanıcının kendi seçicisi, sembolik bağ ya da
bozuk işaret varsa dokunmaz). `--no-sharepicker` atlar, `--dry-run` planı gösterir, `--uninstall`
yalnızca bloğu kaldırır. Ayrıntı: `docs/design/screenshare-picker.md`.

**Kurulum ve güncelleme ayrı; güncelleme ayarları korur (kullanıcı isteği 2026-10-16).** install.sh
kurulumdan önce türünü belirler (önceki Nuxut izi: `setup.toml`, `shell.toml` ya da kurulu ikili →
güncelleme) ve sonda bildirir. Güncellemede ayar dosyalarına install.sh dokunmaz; eski biçimi kabuk
ilk açılışta sürüm göçüyle taşır, eski/geçersiz/işe yaramayan satırları onarır (önce yedek, tek
sessiz bildirim; `msg config status`, `msg config undo`; sözleşme `docs/design/config-migrations.md`).
Hoş geldin yalnızca gerçek ilk kurulumda (`setup.toml` yokken) ya da isteyince (`nuxut-settings
--welcome`). `--reset-config` Nuxut'u ilk kez kurulmuş gibi başlatır (aşağıdaki sıra); `--keep-config`
etkisizdir (eski betiklerle uyum). `--dry-run` planı (taşınacak dosyalar, silinecek eski yedekler) IPC
göndermeden gösterir. Derleme başarısızsa hiçbir şey olmaz. `--reset-config` sırası
(ikililer kurulduktan sonra, servis/Hyprland adımlarından önce):

1. Çalışan kabuk kendi yolundan kapanır: etkin `nuxut-shell.service` → `systemctl --user stop`,
   değilse `msg quit` yalnızca hedef örneğe (`NUXUT_SHELL_INSTANCE` aynen geçer). Kabuk çalışıyor
   ama sonra başlatılamayacaksa (`--no-restart`, Wayland yok, `quit` bilmeyen eski kabuk) taze
   kurulum hiç yapılmaz (uyarı).
2. `nuxut-shell look revert` YENİ kurulan ikiliyle (eski ikililerde komut yok; depo biçimi aynı
   `nuxut-apply`). Başarısız/yoksa uyarılır ve snapshot deposu yerinde bırakılır: Nuxut öncesi
   değerleri taşıyan sabitlenmiş snapshot'lar kaybolmaz, yeni ilk açılış onların üstüne yazar.
3. Açık izin listesi `$XDG_STATE_HOME/nuxut/backups/<UTC zaman>/{config,state}/` altına TAŞINIR
   (0700, `BENIOKU.txt`; son 5 yedek kalır, yalnızca bu adlandırmadaki dizinler silinir):
   `~/.config/nuxut/{shell,framework,displays}.toml`, `$XDG_STATE_HOME/nuxut/{setup,palette}.toml`,
   `snapshots/` (yalnızca 2. adım başarılıysa; `shell-state.toml` bilerek listede YOK — güncelleme
   bildirimi sürüm karşılaştırması için kalır). `$XDG_RUNTIME_DIR/nuxut/welcome-<imza>` silinir
   (aynı oturumda Hoş geldin yeniden açılsın). Joker yok: aynı dizinlerdeki eski projenin
   dosyaları, `$XDG_DATA_HOME/nuxut` (takvim hesapları/OAuth, önbellek), günlükler, `apply.lock`
   dokunulmaz. Ayarlar ve takvim ayrı UI durum dosyası tutmaz (yalnızca framework/shell.toml ve
   palette.toml). Hyprland dahil etme satırı, `nuxut.lua`/`nuxut.conf` ve `hypr-source-added`
   kalır: kabuk açılışta dosyayı yeniden üretir; satırı silmek çalışan Hyprland'i kısayolsuz
   yeniden yükletirdi (kaldırmak isteyen: `msg hypr undo`).
4. Kurulum sürer; kabuk aynı yoldan başlatılır (`systemctl --user start` ya da `setsid`), `setup.toml`
   yok → Hoş geldin ilk açılış gibi; birleşik görünüm varsayılanla yeniden uygulanır ve yeni ilk
   snapshot gerçek Nuxut öncesini tutar. Çıktı: `Ayarlar sıfırlandı, yedek: <yol>`.

## Doğrulama

`cargo test`, `cargo clippy -- -D warnings`; görsel değişiklik canlı Hyprland'de çalıştırılıp
`grim` ile ekran görüntüsü alınarak doğrulanır. Makinede kullanıcının eski Quickshell shell'i
çalışıyor: test ederken yalnızca kendi başlattığın PID'i öldür.

### Testlerde depo yolu (2026-10-14)

Testler depo/paket yolunu ÇALIŞMA ZAMANINDA `std::env::var_os("CARGO_MANIFEST_DIR")`'den çözer
(`cargo test` test sürecine verir); derleme anındaki `env!("CARGO_MANIFEST_DIR")` yalnızca ikili
cargo dışında çalıştırılınca yedektir, çözülen yol yoksa açık mesajla panik. Neden: cargo, paketin
mutlak yolu değişince (worktree taşınması, sembolik bağlı kum havuzu, `--manifest-path`) test
ikilisini yeniden derlemez; gömülü yol ölü ya da BAŞKA bir worktree'nin ağacını gösterir (e413de1:
pam_exec ENOENT). Kabuk testleri ortak `tests/support/repo.rs`'i (`manifest_dir`, `repo_path`)
kullanır; ayrı crate'ler (`polkit`, `nuxut-wallpapers`, `nuxut-pam`, `lock`) aynı kalıbı küçük bir
yerel fonksiyonla uygular. Kaynak tarayan bekçiler (`no_hardcoded_*`, `mascot_placement`, i18n
`source_guard`) taranan `.rs` sayısı `MIN_SHELL_SOURCES`'un altındaysa başarısız olur: boş/yanlış
ağaçta sessiz yeşil yok.

### Testlerde zaman: koşul bekle, süre varsayma (2026-10-17)

**Hata:** `audio_service`, kamera (`unattributed_and_renamed_opens_fail_closed`) ve tepsi testleri
iniş turlarında ara sıra düşüyordu. **Kök neden:** kısa duvar saati sınırları: PipeWire
araçlarına `timeout 5`, kamera/tepsi/takvim beklemelerine 3–5 sn, "bu sürede gelenleri topla"
pencereleri (tepsi 1.5 sn), "hızlı kapandı" iddiaları (0.5–2 sn), tanı raporunun ürün içi 900 ms
`wpctl` sınırı. Testler derleme slice'ında düşük öncelikle ve CPU kotasıyla çalışır; bellek bekçisi
(`mem-watch`) baskıda slice'ı ~1 sn'lik art arda dondurur. Donan süreç (test, özel
PipeWire/D-Bus, servis thread'i) çözüldüğünde son tarih çoktan geçmiş olur. **Kanıt:** turun
scope'u 1–1.5 sn dondurulup 0.2–0.8 sn çözülerek (bekçi benzetimi) eski ikililerle 6 turun 2'si
kırmızı: tanı raporu `(wpctl zaman aşımı)` ve tepsi sınır testi 63/64 öğe. Yalnız CPU yarışında
(tek çekirdeğe sıkıştırma, ~10× yavaşlama) eski sürüm de yeşil: ses ve kamerada kök neden süre
sınırı, ürün yarışı değil.

**Gerçek ürün hatası (tepsi, düzeltildi):** sınırlar genişletilince donma benzetimi tepside
cevapsız menü açılışlarını ortaya çıkardı: önceki ağaç yokken menünün ilk `GetLayout` okuması
zaman aşımına uğrarsa (donma, meşgul uygulama) hiçbir şey yayınlanmıyor, yeniden de
denenmiyordu; tıklanan tepsi menüsü hiç açılmıyordu. Artık ilk okuma öğe okumasıyla aynı bütçe
ve geri çekilmeyle (`initial_read_attempts`, `retry_delay`) yeniden denenir, `AboutToShow`
tekrarlanmaz. Test: `first_menu_read_timeout_is_retried` (düzeltmesiz sürümde menü hiç gelmez).
Test seçenekleri ürün varsayılanına çekildi (çağrı 2 sn, 4 deneme; eskiden 400 ms × 2).

**Doğrulama:** aynı donma benzetimiyle (her turda tam lib paketi yük olarak + 2× `audio_service` +
2× kamera + tepsi paketi paralel, derleme slice'ında) yeni kodla 20/20 tur yeşil.

**Kural:**
- Beklemeler olay/koşul bekler; üst sınır tek kaynaktır: `tests/support/liveness.rs`
  `LIVENESS` (120 sn; entegrasyon testlerinde `support::WAIT`, birim testlerinde
  `crate::test_liveness::LIVENESS`). Başarılı koşunun süresi bundan etkilenmez.
- "Takılmadan bitti" iddiaları `PROMPT` (10 sn) ile ölçülür; koruduğu durum (takılı D-Bus
  çağrısı, sonsuz bekleyen thread) bunun çok üstünde kalacak şekilde kurulur (ör. tepsi
  `unresponsive_apps_cannot_block_the_service`: çağrı zaman aşımı `3 × PROMPT`).
- Sabit süreli toplama yalnız "şu olmamalı" denetimi içindir (kısa pencere; donma onu yalnız
  zayıflatır, kırmaz). "Şu gelmeli" bekleyen her şey koşul bekler (`collect_until`, `eventually`).
- Dış araç çağrıları (`pw-dump`, `wpctl`…) kendi süre sınırını `LIVENESS`'tan alır; sorgu
  döngüsündeki geçici başarısızlık yeniden denenir.
- Bilinen sınır (ürün, düzeltilmedi): ses yazımında hedef, gidiş-dönüş bittikten 1 sn
  (`SETTLE`) sonra sunucu değerine bırakılır; kabuk 1 sn'den uzun donup çözülürse yankı
  işlenmeden bırakılabilir ve o anda gelen tek bir göreli adım eski değerden hesaplanır.
  Kendiliğinden düzelir (sonraki yankı), masaüstünde 1 sn'lik donma gerektirir.
