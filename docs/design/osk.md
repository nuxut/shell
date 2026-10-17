# Ekran klavyesi (t125, v1)

Görev: 2026-10-14. Uygulama. Bu belge kararları, gerekçeleri ve
bilinen sınırları tutar; mimari özeti `docs/ARCHITECTURE.md` "Ekran klavyesi".

## Ne yapar

Fare ya da dokunmayla yazmak için ekranın alt kenarında bir klavye. Tuşlar
`zwp_virtual_keyboard_v1` ile **odaklı istemciye** gider (fiziksel klavyeyle aynı yol: kısayollar,
ölü tuşlar, tuş tekrarı istemcinin kendi mantığıyla çalışır).

## Kararlar

### Girdi: kendi keymap'imiz, evdev kodları

- Sanal klavyeye her zaman kabuğun **gömülü** keymap'i gönderilir (`assets/osk/{tr,us}.xkb`); tuş
  basışları o keymap'in evdev kodlarıdır. Ekrandaki etiket aynı keymap'ten xkbcommon ile çıkarılır
  (`services/osk/keymap.rs` `Labels`): görünen karakter, istemcinin aynı tuş + değiştiricilerle
  üreteceğinin ta kendisidir. Elle yazılmış ikinci bir karakter tablosu yok; TR-Q'nun `ı/I`, `i/İ`,
  Caps davranışı ve AltGr seviyesi (`@ € ₺ { [ ] } \ | ~ <  >`, ölü tuşlar) xkeyboard-config'ten
  gelir ve testler gerçek değerleri doğrular.
- Gömülü metin, kullanıcının sistemindeki xkb verisinden **bağımsızdır**: bu makinede
  `symbols/tr` yerel olarak değiştirilmiş (Caps → Super); paketten derlenmemiş bir keymap ekran
  klavyesinde Caps'i bozardı. Bekçi test: `embedded_keymaps_are_stock_not_local_overrides`.
- Neden metin gömülü, çalışma anında derleme değil: kullanıcının `XKB_DEFAULT_*` ortamı ve yerel
  değişiklikleri etiket ile çıktıyı ayırabilirdi; aynı metin hem etikete hem compositor'a gider.
- Yeniden üretim (xkeyboard-config 2.48, libxkbcommon 1.13; çıktı bayt bayt aynı doğrulandı):

  ```sh
  d=$(mktemp -d)
  bsdtar -xf /var/cache/pacman/pkg/xkeyboard-config-2.48-1-any.pkg.tar.zst -C "$d" usr/share/xkeyboard-config-2
  for l in tr us; do
    env -u XKB_DEFAULT_OPTIONS -u XKB_DEFAULT_LAYOUT -u XKB_DEFAULT_VARIANT -u XKB_DEFAULT_MODEL -u XKB_DEFAULT_RULES \
      xkbcli compile-keymap --include "$d/usr/share/xkeyboard-config-2" --rules evdev --model pc105 \
      --layout "$l" --options "" --output-format 1 --drop-unused > "assets/osk/$l.xkb"
  done
  rm -rf "$d"
  ```

### Düzenler: TR-Q ve US

- v1'de iki düzen: Türkçe Q (xkb `tr`) ve ABD İngilizcesi (xkb `us`). Fiziksel tuş tablosu ortak
  (`services/osk/keys.rs`: ISO 105 tuşun yazı bölümü, 5 satır × 15 birim; ok tuşları alt satırda).
- Etkin düzen `[osk] layout`: `auto` (varsayılan) compositor'ın etkin düzenini izler — Hyprland
  `activelayout` olayı (`HyprlandEvent::ActiveLayout`) ve framework'ün `keyboard_layout_changed`'ı
  (wl_keyboard keymap/grup; sway'de de çalışır). "Turkish" ve "Turkish (…)" (F hariç) → TR, diğer
  her şey → US. `tr`/`us` sabitler. Klavyedeki düzen tuşu (TR/US) elle geçer; elle seçim bir sonraki
  compositor düzen değişimine kadar kalır.
- F klavye, diğer diller: v2. Tuş tablosu ve keymap eklemek yeterli (etiketler otomatik).

### Değiştiriciler

- Shift, AltGr, Ctrl **tek vuruşluk**: bir sonraki tuşa uygulanır ve kalkar; ikinci dokunuş iptal.
  Caps **kilit**. Maskeler `modifiers` isteğiyle gider (Shift tuşunun kendisine basılmaz); istemcinin
  xkb durumu etiketle aynı seviyeyi hesaplar.
- 3. seviyesi hiç olmayan düzende AltGr tuşu soluk ve etkisiz; düzen değişince bekleyen AltGr düşer.
  pc105 US'in 3. seviyesi yalnızca `<>` tuşundadır (`|` `¦`); bu yüzden US'te AltGr açık kalır, diğer
  tuşlar AltGr ile aynı harfi verir (xkb davranışı; test `us_layout`).
- Sembolü olmayan seviye (ör. TR'de AltGr+W) etkisiz ve soluk; basış tek vuruşu tüketmez.

### Tuş tekrarı

Basılı tutulan tuş **gerçekten basılı** kalır: işaretçi basınca `key pressed`, bırakınca/tuştan
kayınca/yüzeyden çıkınca `key released`. Tekrarı istemci, kullanıcının Hyprland `repeat_rate` /
`repeat_delay` ayarıyla yapar (fiziksel klavyeyle aynı his; kabukta ikinci bir tekrar zamanlayıcısı
yok). Kabuk düşerse compositor sanal klavyeyi yok eder ve basılı tuşları bırakır; kapanışta
(sinyal, `msg quit`) kabuk tuşları ve değiştiricileri kendisi sıfırlar.

### Yüzey ve "tek geçici yüzey" kuralı

- **v2 (karar 2026-10-16, kullanıcı isteği: "alt kısmı itiyor; alttan yapışık, kavisli ve
  overlay olmalı; tutup çekilebilmeli, floating yapabilmeliyiz"):** ayrı layer-shell yüzeyi
  `nuxut-osk`, **Overlay** katmanı, **klavyenin kendi boyutunda** (yüzerken her yanda tema gölgesi
  payı). Genişlik `15 × 1,25 × key_height` (çerçeve iç alanından dar olamaz, kenarlardan 16 px pay).
  - **Yanaşık (varsayılan):** kabuk çerçevesinin iç alanının altında, ortalı, **exclusive zone
    yok** — pencereler itilmez. Üst köşeler kabuğun panel geometrisiyle yuvarlak (framework
    `BlobField`, ADR-0008: boşluğa açılan köşe `radius_large`; şekil çerçevenin altına taşırılıp
    kırpılır, alt köşeler görünmez); yeni geometri yazılmadı.
  - **Yüzen:** dört köşe yuvarlak + tema gölgesi.
  - **Tek yüzey, iki kip (2026-10-16, framework `set_layer_placement` sonrası):** iki kip de sol üst
    anchor + margin (klavye dikdörtgeninden), exclusive zone `-1` ve gölge paylı aynı boyutu
    kullanır; kip yalnızca biçimi değiştirir. Böylece sürüklerken kip geçişi boyut değiştirmez
    (configure gidiş-dönüşü ve eski boyutlu tamponun yeni yerde bir kare görünmesi yok). Bedel:
    yanaşıkken yüzey yanlarda ve altta gölge payı kadar saydam büyür (girdi bölgesi yalnızca
    klavye; alt pay ekranın/çerçevenin altına taşar). Çıkış boyutu değişince açık klavye yeni
    alana yerleşir (`osk_screen_resized`).
  - Girdi bölgesi yalnızca klavye dikdörtgeni (gölge payı tıklamayı geçirir).
  - Tam ekran saydam yüzey **kullanılmaz** (2026-10-16): Hyprland'in tam ekran
    uygulamalarda direct scanout/VRR'ını bozar, her karede kompozisyon (güç) maliyeti getirir.
- **Tutamaç ve sürükleme:** klavyenin üstünde ince bir şerit (ortada hap). Basıp yukarı
  `UNDOCK_THRESHOLD` (24 px) çekince yüzen kipe geçer; yüzerken tutamaçtan taşınır (kabuk
  çerçevesinin iç alanında kalır); alt kenarı yanaşma çizgisine `SNAP_DISTANCE` (48 px) yakın
  bırakılırsa yeniden yanaşır. Bırakınca tema `spatial` yayıyla yerine oturur. Kip ve konum
  **oturum içinde** hatırlanır (gizleyip açınca aynı yerde; shell.toml'a yazılmaz). Saf mantık
  `services/osk/place.rs` (birim testli).
  - Sürükleme (2026-10-16): klavye yüzeyi her harekette framework
    `Context::set_layer_placement` ile taşınır (yerleşim bir sonraki karenin commit'iyle, içerikle
    aynı anda gider); bırakınca yayın her adımı da aynı yoldan. Yüzey yeniden kurulmaz, ek yüzey
    yok (önceki tam ekran `nuxut-osk-drag` kaldırıldı). Örtük kapma: işaretçi yüzey dışına çıksa da
    olaylar bu yüzeye gelir.
  - **İşaretçi tabanı (bilinen sınır):** basılıyken yüzey taşınırsa yüzey içi konumun tabanı
    compositor'a bağlı (framework ARCHITECTURE "Sürükleme sınırı"). `place::GrabFrame`: sway
    basma anındaki yüzey konumunu tutar (`Press`, izole sway'de ölçüldü); Hyprland yereli yüzeyin
    şimdiki geometrisinden hesaplar (`Live`: taban = son commit'teki konum, `draw`'da kaydedilir;
    Hyprland kaynağındaki `surfaceCoords = mouse - surfaceBox` okumasına dayanır, **canlı
    doğrulanmadı** — kullanıcının oturumuna sanal girdi gönderilemiyor, iç içe Hyprland bu VM'de
    GBM ayırıcısı olmadığı için açılmıyor). v0.56.2 kaynağıyla yeniden okundu (2026-10-16):
    `InputManager::mouseMoveUnified` düğme basılıyken odağı tutar ve her harekette
    `surfaceLocal = mouseCoords - getSurfaceBoxGlobal().pos()` hesaplar; katman yüzeyi commit'te
    `m_realPosition`'u yeni geometriye ışınlar (açılış animasyonu sürmüyorsa) — yani taban son
    işlenen commit'teki konum (`Live`). Kalan küçük sınır: compositor commit'i işlemeden önce
    gönderdiği bir hareket olayı, `draw`'da kaydedilen yeni tabanla okunursa o tek olayda bir
    karelik hareket kadar sapar; sonraki olayda kendiliğinden düzelir (birikmez). Seçim
    `HYPRLAND_INSTANCE_SIGNATURE`'a göre. Yanlışsa belirti: sürüklerken klavye işaretçiden iki
    kat hızlı (taban gerçekte `Press`) ya da yarım hızda/titrek (gerçekte `Live`) gider; alanın
    içine çekildiği için ekrandan çıkmaz. Hyprland'de elle sürükleyerek doğrulanmalı.
  - **Monitörler arası (2026-10-16; kullanıcının makinesi iki monitörlü):** katman
    yüzeyi bir çıkışa bağlıdır (wlr-layer-shell çıkış değiştiremez) ve sürükleme örtük kapmayla
    kaynak yüzeyde sürer; bu yüzden sürüklerken klavye kaynak çıkışın kenarında kalır. Bırakınca
    işaretçi başka bir çıkışın üstündeyse (`Screen::position` + xdg-output düzeni,
    `place::drop_target`) klavye orada **bir kez** yeniden kurulur: işaretçiye göre aynı göreli
    yerde, hedef alana çekili; yüzmeyen (yana sürüklenen) yanaşık klavye orada da yanaşık, alta
    yakın bırakılan yanaşır. Sürükleme boyunca yeniden kurma yok. İzole sway'de iki çıkışla
    (1920×1080 + 1600×1000) doğrulandı: yüzen bırakma beklenen konumda, yanaşık yana sürükleme
    hedefte yanaşık, geri sürüklemede ilk monitöre yanaştı.
  - `[osk] reserve_space = true` yanaşıkken eski yüzeyi kullanır (yalnızca alt anchor, kendi
    boyutu, exclusive zone): sürüklerken kip geçişinde boyut ve exclusive zone değişir (ayrı
    commit; bir karelik sıçrama olabilir) — isteğe bağlı kip için kabul edildi.
  - Fare ve dokunma aynı yoldan: framework birincil dokunmayı işaretçi olarak öykünür
    (`PointerSource::Touch`, fw 2c21982).
- **`[osk] reserve_space`** (varsayılan `false`): açıkken yanaşık klavye kendi yüksekliğinde
  exclusive zone ister (v1 davranışı; pencereler itilir). Varsayılan kapalı çünkü kullanıcı itmeyi
  istemedi ve çoğu yazı alanı ekranın ortasında; açık bırakılması, odaktaki alanın örtülmesini
  sık yaşayanlar (alt kenardaki terminal, sohbet kutusu) için. Yüzen klavye hiç yer ayırmaz.
- **Klavye etkileşimi yok** (`KeyboardMode::None`): klavyeye tıklamak klavye odağını almaz; tuşlar
  zaten odaklı olan istemciye gider. Bu yüzden ana yüzeyin bir parçası (blob yaprağı) **değil**: ana
  yüzey yaprak açıkken `Exclusive`/`OnDemand` odak alır, klavye orada olsaydı tıklamak odağı kabuğa
  çekerdi.
- **Tek geçici yüzey kuralının istisnası (gerekçe):** ekran klavyesi bir yaprak değil, bir girdi
  aygıtıdır. Kabuğun kendi yazı alanlarına (başlatıcı araması, ağ parolası, pano araması) da
  yazılabilmesi için:
  - Klavye açılınca açık geçici yaprak **kapanmaz**; yaprak açılınca klavye **kapanmaz**.
  - Yaprağın odak kapmasına (`hyprland_focus_grab_v1`) klavye yüzeyi de eklenir
    (`Screen::focus_grab_extra`): klavyeye dokunmak "dışarı tıklama" sayılmaz, yaprak kapanmaz;
    odak yaprakta kaldığı için tuşlar yaprağın arama alanına gider.
  - Klavye sabit panel gibi alan da **oymaz** (kabuğun iç alanı değişmez); bkz. Sınırlar.
- Fiyu (maskot) klavyede **görünmez** (karar).
- Görsel: framework parçaları (`Interaction` state layer, `Label`, `IconView`) ve tema token'ları:
  zemin `surface` + `radius_large`; yazı tuşları `surface_bright`, işlev tuşları tonal kap, etkin
  değiştirici `primary`/`on_primary`, etkisiz tuş `disabled_*`; tuş köşesi
  `nested_radius(radius_large, iç boşluk)`. Harfler `TextRole::Title`, işlev tuşları `Label`.
- Erişilebilir ad (2026-10-15): tuşun ekran okuyucu adı sembol değil katalog metnidir
  (`shell-osk-key-*`: "Geri sil", "Büyük harf kilidi", "Düzeni değiştir, şu an TR"); yazı tuşunda
  görünen karakter. AT-SPI: `KeyboardView::access` (`Group` "Ekran klavyesi" → tuş `Button`, değiştiricide
  basılı durum). Tuşlar **eylemsiz** (2026-10-16): AT sanal klavyeyle odaklı
  pencereye yazamaz (ADR-0011 karar 6 ilkesi); bkz. ARCHITECTURE "Ekran okuyucu".

### Kontroller

- Hızlı ayar kartı `osk` (anahtar + "Açık · Türkçe Q"); compositor protokolü sunmuyorsa gizli.
  Varsayılan panelde **yok** (büyüteçle aynı karar: widget ekleme menüsü ve Ayarlar ›
  Erişilebilirlik).
- `nuxut-shell msg osk toggle|show|hide [--output <ad>]` (hedef: `--output` ya da odaklı monitör;
  başka monitörde açıkken `show` oraya taşır), `msg osk status` → sözleşme **osk-v1**:
  `{"available":bool,"visible":bool,"layout":"tr"|"us","output":"DP-1"|null,"placement":"docked"|"floating",
  "position":[x,y]|null}` (`placement`/`position` 2026-10-16 eklendi; yeni anahtar, eski okuyucu etkilenmez;
  `position` yüzen klavyenin sol üstü, çıkış mantıksal koordinatı).
- Kısayol önerisi `osk_toggle` = **SUPER+ALT+K** (`Plain`; düzenden bağımsız harf, büyüteç
  kısayollarıyla aynı SUPER+ALT ailesi; Hyprland örnek ayarıyla çakışmaz — keybinds testi).
  Hyprland temel kısayol katmanıyla eşgüdümlü (çift tanım yok).

### Ayarlar sözleşmesi (Ayarlar'a)

`[osk]` şeması (since 2), Ayarlar › Erişilebilirlik › "Ekran klavyesi" altında önerilir:

| Anahtar | Tür | Varsayılan | Not |
|---|---|---|---|
| `osk.layout` | choice `auto`/`tr`/`us` | `auto` | etiketler `shell-schema-osk-layout-*` |
| `osk.key_height` | int 32..=72 px | 48 | klavyenin boyu ve genişliği bundan |
| `osk.reserve_space` | bool | `false` | yanaşıkken pencereleri klavyenin üstüne it (etiket `shell-config-osk-reserve-space-*`) |

Göster/gizle bir ayar değil, çalışma anı durumudur (`msg osk show|hide`, `status.visible`).

## Sınırlar (v1)

- **Overlay odaktaki yazı alanını örtebilir** (v2, varsayılan): pencereler itilmediği için alt
  kenardaki yazı alanı (terminal istemi, sohbet kutusu) klavyenin altında kalabilir. Çözüm
  kullanıcıda: klavyeyi yüzdürüp kenara çekmek ya da `[osk] reserve_space = true`. Odaktaki
  alanın konumunu bilmek (text-input imleç dikdörtgeni) compositor'dan istemciye gelmediği için
  otomatik kaçınma yok.
- Yanaşık klavyenin sürükleme başlangıcındaki yeri kabuğun çerçeve iç alanından hesaplanır;
  başka bir panel (waybar vb.) alt kenarda yer ayırmışsa klavye yüzmeye geçerken birkaç piksel
  sıçrayabilir.
- **Kilit ekranında çalışmaz.** Kilit `nuxut-lock` ayrı süreçtir ve session-lock yüzeyleri her
  layer'ın üstündedir; kabuğun klavyesi orada görünmez. v2: kilit ikilisinin içinde (kendi
  yüzeyinde, aynı `services::osk` çekirdeği) — parolanın kabuk sürecinden geçmemesi için.
- **Bar alttayken** alt kenardan büyüyen yapraklar (başlatıcı, alt çekmece) klavyenin altında
  kalabilir: klavye kabuğun iç alanını oymaz. v2: açık klavyeyi alt kenarda iç alanı oyan bir şerit
  olarak saymak (sabit panel gibi).
- Dokunmatik ekran: framework runtime'ı `wl_touch` olaylarını işaretçi olayı olarak vermiyorsa
  dokunma çalışmaz; v1 işaretçiyle doğrulandı (touch → framework talebi).
- Tek düzen grubu: keymap'ler tek gruplu; "düzen değiştir" kabuğun kendi keymap'ini değiştirir,
  Hyprland'in `kb_layout` grubunu değiştirmez.
- Esc, F tuşları, Super yok (v2: ikinci sayfa).
- Tahmin, otomatik büyük harf, uzun basışta aksanlı seçenek yok.
- socket2 satır tabanlıdır ve Hyprland pencere başlığındaki satır sonunu kaçırmaz: başlığına
  `\nactivelayout>>x,Turkish` koyan bir uygulama sahte düzen olayı üretebilir (widgets bulgusu,
  2026-10-16). Etkisi sınırlı: `auto` iken klavyenin gösterdiği düzen değişir, ama etiket ve
  gönderilen keymap birlikte değiştiği için yazılan karakter her zaman etiketle aynıdır; parola
  ya da başka veri sızmaz. Olay adı yalnızca satır başından okunur; başka olayların verisindeki
  `activelayout>>` metni sayılmaz (test `other_events_carrying_activelayout_text_are_not_layout_changes`).
- Kabuk düşerse ve compositor sanal klavyeyi temizlemezse basılı tuş kalabilir (wlroots/Hyprland
  istemci kopunca temizler).

## Testler

- Birim: `services/osk/keys.rs` (satır genişlikleri, 48 yazı tuşu, özel tuş kodları),
  `services/osk/keymap.rs` (gerçek xkb'den TR-Q harf/Shift/Caps/AltGr, ölü tuş etiketi, US, keymap
  bekçisi), `services/osk/mod.rs` (Emit sırası: keymap → maskeler → tuş, tek vuruş, kilit, düzen
  geçişi, sıfırlama, config, status sözleşmesi), `osk_view.rs` (yerleşim, bas-tut-kay-bırak,
  etkisiz AltGr), `ipc.rs` (`osk` komutları), keybinds (`osk_toggle`).
- Entegrasyon: `tests/it/hyprland_service.rs` `active_layout_is_forwarded_without_refresh` (sahte
  socket2; virgüllü düzen adı, bozuk veri atlanır, durum sorgusu yok).
- Canlı: izole headless sway'de kendi `kitty --class nuxut-test` penceresine yazma (sway
  `zwp_virtual_keyboard_v1` sunar); ekran görüntüsü yalnız klavye yüzeyinden.
