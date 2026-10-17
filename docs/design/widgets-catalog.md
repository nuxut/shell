# Widget kataloğu: bugün olanlar ve yeni adaylar

Durum: P1 onaylandı (2026-10-15); uygulama durumu ve kararlar §6'da. İlk öneri (2026-10-15; istek:
kullanıcı "widget sayısı artabilir, ama gerçekten useful olanlar"). Adaylar kullanıcıya gösterilir;
**onaysız uygulama yok**. Onaylanan her aday
kendi `docs/design/<ad>.md` belgesiyle uygulanır.

Ölçütler: (1) kişi bunu her gün ya da kritik bir anda gerçekten kullanır mı; (2) veri kaynağı
mevcut servis mi, yeni servis mi; (3) **boşta sıfır iş** — görünmüyorken ya da veri değişmiyorken
uyanma, yoklama, yeniden çizim yok; olay güdümlü kaynak tercih edilir; (4) bar şeridi dar ve
değerlidir: bar'a yalnızca "bir bakışta" bilgisi ya da tek tık eylemi olan girer, kalanı panel.

## 1. Bugün olanlar

### Bar

| Widget | Ne | Veri |
|---|---|---|
| `logo` | Marka işareti; tık: başlatıcı | — |
| `launcher` | Başlatıcı düğmesi | launcher kataloğu |
| `workspaces` | Workspace yuvaları (ikon/numara/nokta), tekerlekle geçiş | Hyprland socket2 |
| `active_window` | Odaklı pencere başlığı | Hyprland |
| `clock` | Saat; tık: takvim popout'u (etkinlikler) | saat + `nuxut-calendard` |
| `now_playing` | Kapak · "Başlık · Sanatçı" · durum (2026-10-15, yeni) | MPRIS (`services::media`) |
| `tray` | StatusNotifierItem tepsisi, dbusmenu | `services::tray` |
| `sysmon` | CPU/bellek/GPU/sıcaklık/ağ hızı/disk; tık: ayrıntı popout'u | `services::sysmon` |
| `status` | Ağ · Bluetooth · ses · pil ikon grubu; tık: hızlı ayarlar | NM, BlueZ, PipeWire, UPower |
| `notifications` | Okunmamış sayacı; tık: bildirim merkezi | bildirim sunucusu |
| `session` | Oturum düğmesi; tık: oturum kartı | logind |
| `panel-toggle` | Seçilen paneli açan düğme | — |

### Kenar panelleri

| Widget | Ne |
|---|---|
| `session` | Kullanıcı, kilitle/çıkış/uyku/yeniden başlat/kapat |
| `notifications` | Bildirim merkezi + Rahatsız Etmeyin anahtarı |
| `volume` | Çıkış ve giriş: sessiz + seviye |
| `brightness` | Ekran başına parlaklık |
| `night_light` | Gece ışığı anahtarı + sıcaklık |
| `magnifier` | Büyüteç anahtarı + çarpan |
| `power` | Güç profili, pil durumu |
| `network` | Wi-Fi listesi, bağlan/kes, kablolu |
| `bluetooth` | Aç/kapa, cihazlar, bağlan/kes, eşleştirme onayı |
| `media` | Kapak, başlık, ilerleme, önceki/oynat/sonraki |
| `calendar` | Ay takvimi + seçili günün etkinlikleri |

Bar dışı ama ilgili: OSD (ses/parlaklık), bildirim popup'ları, başlatıcının pano ve emoji
kipleri, ekran görüntüsü, ekran paylaşım seçicisi, çıkarılabilir medya bildirimleri.

## 2. Kıyas

- **GNOME:** üst bar'da saat (popout'ta takvim etkinlikleri, hava, dünya saatleri, RE anahtarı),
  durum menüsü (ağ/ses/pil), **gizlilik göstergeleri** (mikrofon/kamera/ekran paylaşımı turuncu),
  **ekran kaydı göstergesi** (kırmızı, süre, tıkla durdur), **klavye düzeni** göstergesi;
  hızlı ayarlarda RE/güç modu/gece ışığı/koyu tema/"uyanık tut" eklentileri.
- **KDE Plasma:** sistem tepsisinde pano geçmişi, **uygulama başına ses**, **klavye düzeni**,
  **Diskler ve Aygıtlar** (güvenle çıkar), **güncelleme bildirici** (Discover), medya oynatıcı,
  RE, hava (eklenti), KDE Connect; görev yöneticisi ve sayfalayıcı.
- **macOS:** Denetim Merkezi (Wi-Fi, BT, **Odak/RE**, ekran, ses, **Şimdi Çalıyor**); menü
  çubuğunda **turuncu/yeşil gizlilik noktaları**, **giriş kaynağı (klavye)** menüsü, ekran kaydı
  göstergesi; masaüstü/bildirim merkezi widget'ları: takvim (sıradaki etkinlik), hava, anımsatıcılar,
  borsa.
- **Windows 11:** tepside ağ/ses/pil, **mikrofon/konum kullanımda** simgesi, **dil/klavye**
  değiştirici, **"Donanımı güvenle kaldır"**; hızlı ayarlar (Odak, gece ışığı, uçak modu, erişilebilirlik,
  yayınla); bildirim merkezinde takvim + **odak zamanlayıcısı**; Widget panosu (hava, borsa, haber).
- **Referanslar:** *noctalia* bar widget'ları: active_window, audio_visualizer, battery, bluetooth,
  brightness, **caffeine/idle_inhibitor**, **clipboard**, clock, control_center, **custom_button**,
  **keyboard_layout**, **lock_keys**, launcher, **media**, network, nightlight, notification,
  power_profile, **privacy**, **screenshot**, session, settings, spacer, sysmon, **taskbar**, text,
  **theme_mode**, tray, volume, wallpaper, **weather**, workspaces. *caelestia-shell*: bar'da az
  (OS ikonu, workspace, pencere, saat, durum ikonları, tepsi, güç); dashboard'da medya, performans,
  **hava**; utilities'te kayıt kartı/toast'lar. *dots-hyprland (ii)*: bar'da medya, kaynaklar,
  pil, **klavye düzeni (HyprlandXkbIndicator)**, **hava**, okunmamış sayacı, **yardımcı düğmeler**
  (ekran kırpma, **ekran kaydı**, renk seçici, ekran klavyesi, **mikrofon sessiz**, **koyu mod**,
  performans profili); sağ yan panelde hızlı anahtarlar, **ses karıştırıcı**, **pomodoro**, **todo**,
  takvim, gece ışığı, Wi-Fi/BT listeleri; sol yan panelde sohbet asistanı, çevirmen, anime.

Ortak payda (dört masaüstünün de sunduğu): gizlilik göstergesi, klavye düzeni, RE/odak, ses
karıştırıcı, çıkarılabilir aygıt, takvimde sıradaki etkinlik. Nuxut'ta bunların veri kaynaklarının
çoğu **zaten var** (aşağıda).

## 3. Adaylar

Maliyet sütunu: **B** = bellek, **U** = uyanma, **C** = boşta CPU. "0" = olay güdümlü, görünmezken
iş yok.

### P1 — gerçekten işe yarar, veri hazır ya da çok ucuz

| # | Aday | Ne işe yarar / kim kullanır | Veri kaynağı | Maliyet | Yer |
|---|---|---|---|---|---|
| 1 | **Gizlilik göstergesi** (`privacy`) | Mikrofon, kamera ya da ekran paylaşımı kullanımdayken bar'da renkli ikon (yalnız o an görünür); ipucunda hangi uygulama; tık: ses kartı / paylaşımı durdur. Herkes; güven ve gizlilik ilkesinin görünen yüzü. | Mikrofon: `AudioState::recording` (PipeWire akışları, **var**). Ekran paylaşımı: Hyprland `screencast>>` olayı (bugün yok sayılıyor; tek satır) + paylaşım seçicisi. Kamera: `/dev/video*` üzerinde inotify `IN_OPEN`/`IN_CLOSE` (yeni küçük izleyici; yoklama yok). | B ~0, U 0, C 0 | bar |
| 2 | **Klavye düzeni** (`keyboard_layout`) | Etkin düzen ("TR", "US"); tık: sıradaki düzen, tekerlek: geçiş; yalnız 2+ düzen varken görünür. Çok dilli herkes (bu makinede tr + us). | Hyprland `activelayout>>` olayı (bugün yok sayılıyor) + `[input] keyboard.layouts`; geçiş `hyprctl switchxkblayout`. | 0 / 0 / 0 | bar |
| 3 | **Hızlı anahtarlar kartı** (`toggles`) | Tek dokunuşluk anahtar ızgarası: Rahatsız Etmeyin, Uyanık tut, Koyu/açık tema, Gece ışığı, Wi-Fi, Bluetooth, (Mikrofon sessiz). GNOME/macOS/Windows hızlı ayarlarının çekirdeği; bugün bunlar ayrı kartlara dağılmış ya da yalnız `msg` ile erişilebilir. | Hepsi **var**: `notifications.dnd`, `idle` inhibit (`msg idle inhibit`), `look` (tema kipi), `night_light`, NM, BlueZ, ses. | 0 / 0 / 0 | panel |
| 4 | **Uyanık tut** (`caffeine`) | Ekranın kararmasını/kilitlenmesini geçici olarak engeller; bar'da yalnız etkinken görünen fincan ikonu + tık ile aç/kapa. Sunum, video, uzun derleme izleyen herkes. | `services::idle` inhibitor (**var**). | 0 / 0 / 0 | bar (+ #3 içinde) |
| 5 | **Sıradaki etkinlik** (`next_event`) | "14:00 Ekip toplantısı · 12 dk" — yalnız eşik (ör. 2 sa) içinde etkinlik varken görünür; tık: takvim popout'u. Takvim kullanan herkes; Nuxut Takvim ile bütünleşmenin bar'daki yüzü. | `services::calendar` (`nuxut-calendard`, **var**). | B küçük; U: yalnız etkinlik eşik içindeyken dakikada bir (saat vuruşu zaten var), C 0 | bar |
| 6 | **Ses karıştırıcı** (`mixer`) | Uygulama başına ses/sessiz, çıkış cihazı seçimi. Toplantıda müziği kısan, tarayıcıyı susturan herkes; KDE/Windows/GNOME eklentisi standardı. | `AudioState::playback` akışları (**var**), seviye yazımı mevcut `writes` yolu. | 0 / 0 / 0 | panel |
| 7 | **Ekran kaydı göstergesi** (`recording`) | Kayıt sürerken kırmızı nokta + geçen süre; tık: durdur. Kayıt özelliği varsa **zorunlu** (yoksa kullanıcı kaydı nasıl durduracağını bilemez). | Kayıt (screenshot.md "Kayıt v1.1", henüz yok) — **kayıt özelliğiyle birlikte**. | U: yalnız kayıtta saniyede bir | bar |

### P2 — işe yarar, ama yeni servis/ağ ya da kitle daha dar

| # | Aday | Ne işe yarar / kim kullanır | Veri kaynağı | Maliyet | Yer |
|---|---|---|---|---|---|
| 8 | **Çıkarılabilir aygıtlar** (`drives`) | USB takılıyken ikon; tık: aygıt listesi, aç / güvenle çıkar. Windows/KDE standardı; bugün yalnız bildirimden erişiliyor. | `services::removable_media` (UDisks2, **var**). | 0 / 0 / 0 | bar (yalnız aygıt varken) |
| 9 | **Hava durumu** (`weather`) | Sıcaklık + ikon; popout'ta saatlik/günlük. Geniş kitle, bütün referanslarda var. | **Yeni servis:** Open-Meteo (anahtarsız), `nuxut-net` ile yalnız https; konum elle (şehir/koordinat) — IP konumu yok (gizlilik). | B küçük; U 30 dk'da bir (yalnız widget varken), ağ | bar + panel |
| 10 | **Güncellemeler** (`updates`) | Bekleyen paket sayısı; tık: terminalde güncelle / Nuxut güncelleyicisi. Arch kullanıcıları ister. | **Yeni:** `checkupdates` (pacman-contrib; geçici db, root yok) + Nuxut sürüm kanalı. | U 6 sa'te bir alt süreç + ağ; widget yoksa hiç | bar (yalnız >0) |
| 11 | **Odak zamanlayıcısı** (`timer`) | Pomodoro / geri sayım; bittiğinde bildirim ve RE'yi isteğe bağlı açma. Windows "Odak", dots-hyprland pomodoro. | **Yeni**, küçük; durum yalnız bellekte. | U: yalnız çalışırken saniyede bir (bar metni) | panel (+ bar'da çalışırken kalan süre) |
| 12 | **VPN göstergesi** | VPN/WireGuard bağlıyken kilit ikonu; `status`'a yeni gösterge olarak. | NM modelinde VPN türü **var** (`nuxut-nm`). | 0 | bar (`status` içine) |
| 13 | **Görev çubuğu** (`taskbar`) | Açık pencereler (ikon), tık: odak/küçült. Windows/GNOME'dan gelen yeni kullanıcı için tanıdık; tiling kullanıcısı için ikincil. | Hyprland anlık görüntüsü (**var**) + uygulama ikonları. | 0 | bar |
| 14 | **Pano düğmesi** (`clipboard`) | Başlatıcının pano kipini açar; fareyle çalışanlar için kısayolun görünür karşılığı. | Mevcut pano geçmişi. | 0 | bar |

### P3 — dar kitle ya da mevcut yolla zaten karşılanıyor

| Aday | Not |
|---|---|
| Not / yapılacaklar kartı | Panelde kısa not; metin düzenleme + kalıcı depolama gerekir. Takvim uygulamasının görevleri gelirse oraya bağlanmalı. |
| Ekran görüntüsü düğmesi | Kısayol + başlatıcı karşılıyor; yalnız fareyle çalışan için. |
| Renk seçici | Tasarımcı/geliştirici; başlatıcı komutu daha uygun. |
| Kilit tuşları (Caps/Num) | LED'siz dizüstü için; güvenilir veri için evdev erişimi (input grubu) gerekir. |
| Özel komut widget'ı (waybar `custom`) | Güç kullanıcısı; kullanıcı komutu yoklama demek — ancak alt sınırlı aralık ve zaman aşımıyla. |
| Dünya saatleri | Saat popout'una ek satır olarak daha doğru (ayrı widget değil). |
| Ayırıcı / boşluk | Yerleşim aracı; bölümler (start/center/end) çoğu ihtiyacı karşılıyor. |

## 4. Eleme (önerilmiyor)

| Aday | Gerekçe |
|---|---|
| Ağ hızı (ayrı widget) | `sysmon`'un `network` seçeneği zaten ↓↑ hız gösteriyor; kopya. |
| Pil (ayrı widget) | `status` pil göstergesi ve güç kartı var. |
| Kripto / borsa | Dar kitle, sürekli ağ yoklaması, finansal veri kaynağı (anahtar, kota, lisans); bar'da süs. |
| Ses görselleştirici (cava) | Sürekli ses örnekleme + her karede çizim: "boşta sıfır iş"in tersi, bilgi taşımaz. |
| Haber / RSS akışı | Dikkat dağıtır, ağ yoklar; bir masaüstü kabuğunun işi değil. |
| Sohbet asistanı / çevirmen / anime (dots-hyprland sol panel) | Ayrı uygulama işi; kabuğa dış servis anahtarı ve büyük yüzey getirir. |
| Duvar kağıdı / tema değiştirici düğmesi | Ayarlar'ın işi; koyu/açık geçişi #3 içinde yeterli. |
| Sistem bilgisi (OS sürümü, çekirdek) | Bir kez bakılır; Ayarlar "Hakkında" sayfası yeri. |

## 5. Önerilen sıra

1. **#1 Gizlilik** ve **#2 Klavye düzeni** — veri hazır (iki Hyprland olayı bugün yok sayılıyor),
   her gün görünür fayda, maliyet sıfır.
2. **#3 Hızlı anahtarlar** (içinde #4 Uyanık tut) — dağınık yetenekleri tek yere toplar; bar
   ikonu olarak #4.
3. **#6 Ses karıştırıcı** ve **#5 Sıradaki etkinlik**.
4. **#8 Çıkarılabilir aygıtlar**, **#12 VPN** (küçük, mevcut veriler).
5. Kullanıcı onayıyla: **#9 Hava**, **#10 Güncellemeler**, **#11 Zamanlayıcı**, **#13 Görev çubuğu**.
6. **#7 Kayıt göstergesi** ekran kaydı özelliğiyle aynı iş paketinde.

Her yeni bar widget'ı `now_playing` kalıbını izler: görünmezken uzunluk 0, mümkünse sabit
genişlik (bar hasarı yalnız widget dikdörtgeni), saat vuruşu yalnız gerektiğinde, ayarlar
`WidgetKind::settings_schema` ile Ayarlar'da otomatik görünür, `since` ile "Yeni" rozeti.

## 6. Uygulama durumu ve kararlar (2026-10-16)

Kullanıcı P1 adaylarını onayladı (2026-10-15) ve widget/ürün ayrıntılarını uygulamaya bıraktı.
Kararlar gerekçeleriyle ilgili belgelerde:

| Aday | Durum | Belge |
|---|---|---|
| #1 Gizlilik | uygulandı | `privacy-indicator.md` |
| #3 Hızlı anahtarlar, #4 Uyanık tut | uygulandı | `quick-toggles.md` |
| #6 Ses karıştırıcı (`mixer`) + cihaz seçici (ses kartında) | uygulandı | `mixer.md`, `audio-devices.md` |
| #2 Klavye düzeni (`keyboard_layout`) | uygulandı (yalnız 2+ düzende görünür) | `keyboard-layout.md` |
| #5 Sıradaki etkinlik (`next_event`) | uygulandı | `next-event.md` |
| Büyüteç bar hapı + OSD (kullanıcı geri bildirimi) | uygulandı | `magnifier.md` "Bar hapı ve OSD" |
| #7 Kayıt göstergesi | gizlilik göstergesi ekran yakalamayı kırmızıyla gösteriyor; ayrı widget kayıt özelliğiyle | `privacy-indicator.md` |
