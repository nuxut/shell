# Dosya seçici (`nuxut-filechooser`, portal `FileChooser` arka ucu)

(t126 ikinci yarısı, görev). İşçi, 2026-10-15.
İlk yarı: ekran paylaşım seçicisi (`docs/design/screenshare-picker.md`). Portal arka ucunun genel
yapısı: `docs/design/portal.md`.

Bir uygulama (Firefox, Chromium, GTK/Qt uygulamaları, Flatpak, Nuxut Ayarlar) dosya açmak ya da
kaydetmek istediğinde `org.freedesktop.portal.FileChooser`'ı çağırır; xdg-desktop-portal (xdp)
isteği seçili arka ucun `org.freedesktop.impl.portal.FileChooser` arayüzüne iletir. Nuxut'un arka
ucu bu arayüzü sunar ve framework bileşenleriyle çizilmiş kendi seçicisini gösterir.

## Arayüz (xdg-desktop-portal 1.22, `org.freedesktop.impl.portal.FileChooser`)

Kaynak: `/usr/share/dbus-1/interfaces/org.freedesktop.impl.portal.FileChooser.xml` (kurulu paket).

| Yöntem | Girdi | Sonuç (`a{sv}`) |
|---|---|---|
| `OpenFile(o handle, s app_id, s parent_window, s title, a{sv} options)` | `accept_label` s, `modal` b, `multiple` b, `directory` b, `filters` a(sa(us)), `current_filter` (sa(us)), `choices` a(ssa(ss)s), `current_folder` ay | `uris` as, `choices` a(ss), `current_filter` (sa(us)) |
| `SaveFile(…)` | `accept_label`, `modal`, `filters`, `current_filter`, `choices`, `current_name` s, `current_folder` ay, `current_file` ay | `uris` (tam bir öğe), `choices`, `current_filter` |
| `SaveFiles(…)` | `accept_label`, `modal`, `choices`, `current_folder` ay, `files` aay | `uris` (`files` ile aynı sıra), `choices` |

Yanıt kodu: 0 seçildi, 1 kullanıcı vazgeçti, 2 diğer (hata). `handle` yolunda
`org.freedesktop.impl.portal.Request` nesnesi (`Close()`) sunulur; xdp istemci isteği kapatınca
`Close` çağırır → seçici kapanır, yöntem 1 döner.

Seçenek kuralları (arka uç tarafı; spec + GTK arka ucunun davranışı):

- **Filtre:** kural türü 0 = glob, 1 = MIME. `current_filter` listede varsa o seçili açılır;
  liste boşken verilirse tek ve sabit filtre olur; listede yoksa listeye eklenir (GTK gibi).
  Sonuçta seçili filtre (ad + kurallar) döner.
- **Seçim kutuları (`choices`):** seçenek listesi boşsa onay kutusu (`"true"`/`"false"`), değilse
  açılır seçim. Sonuçta yalnızca sunulan kimlikler ve geçerli değerler döner.
- `accept_label`'daki mnemonik alt çizgi atılır (`_Open` → `Open`; `__` → `_`).
- `SaveFile` `current_file` verilmişse klasör onun dizini, ad onun adıdır (`current_name`'i ezer).
- `SaveFiles`: kullanıcı klasör seçer; her ad klasöre eklenir. Klasörde aynı ad varsa benzersiz
  ad üretilir (`rapor.pdf` → `rapor (2).pdf`; uzantıdan önce; karar aşağıda).
- `directory` (yalnız OpenFile): yalnızca klasörler seçilir; kabul düğmesi seçili klasörü, seçim
  yoksa bulunulan klasörü döndürür.

## Süreç mimarisi: arayüz mevcut arka uçta, görünüm istek başına ayrı süreç

| Seçenek | Başlangıç maliyeti | Boşta | Güvenlik / dayanıklılık |
|---|---|---|---|
| A. Her şey `xdg-desktop-portal-nuxut`'ta | yok (süreç zaten açık) | portal renderer, EGL, font veritabanı yükler: boşta onlarca MB, GPU bağlamı; Settings portalının "UI'sız küçük ikili" sözleşmesi bozulur | UI çökmesi (sürücü, font) görünüm portalını da düşürür; GTK/Qt uygulamaları koyu kipi kaybeder |
| B. Ayrı, sürekli açık seçici servisi | düşük | sürekli açık ikinci süreç, ikinci bus adı/`.portal` kaydı | yalıtım iyi ama boşta iş ve bellek |
| **C. Arayüz portalda, görünüm istek başına `nuxut-filechooser` süreci** | süreç + EGL + font (ölçüm aşağıda) | **sıfır**: portal yalnızca çağrı gelince çocuk başlatır; çocuk sonuçla çıkar | UI çökmesi yalnızca o isteği 2 ile bitirir; portal küçük kalır; eşzamanlı istekler bağımsız |

**Karar: C** (2026-10-15). `xdg-desktop-portal-nuxut` aynı bus adında ikinci arayüz
olarak `FileChooser`'ı sunar (tek `.portal` kaydı, tek etkinleştirme); her çağrıda
`$LIBEXEC/nuxut-filechooser`'ı başlatır, sonucu doğrulayıp URI'ye çevirir. Ekran paylaşım
seçicisiyle aynı kalıp (kısa ömürlü UI süreci, sözleşme stdin/stdout).

### Portal ↔ seçici sözleşmesi (`nuxut_portal::chooser`)

- Portal çocuğu argümansız başlatır; ikili yolu: `NUXUT_FILECHOOSER_BIN` (yalnızca test/geliştirme)
  → çalışan portal ikilisinin dizini (`$LIBEXEC`). Ortam portalınkidir (systemd kullanıcı ortamı:
  Hyprland `WAYLAND_DISPLAY`'i oraya aktarır; xdph ve gtk arka ucu da buna dayanır).
- **stdin:** tek satır JSON istek (`ChooserRequest`), ≤ 1 MiB; portal stdin'i açık tutar.
  **stdin'in kapanması iptaldir**: portal `Close` alınca ya da portal ölünce (bus kapandı, çöktü)
  boru kapanır, seçici penceresini kapatıp 1 ile çıkar — yetim seçici kalmaz. Seçici 2 sn içinde
  çıkmazsa portal `SIGKILL` yollar.
- **stdout:** yalnızca tek satır JSON sonuç (`ChooserResult`), ≤ 8 MiB; loglar stderr'e (journal).
  Çıkış kodu: 0 seçim, 1 vazgeçildi, 2 hata. Portal sonucu yalnızca 0'da ve geçerliyse kullanır.
- Yollar sözleşmede **yüzde kodlu bayt dizisi** (`file://` URI biçimi): UTF-8 olmayan dosya adları
  kayıpsız taşınır (JSON dizesi UTF-8 olmak zorunda).
- Portal sonucu ayrıca doğrular: yol mutlak, NUL yok, `.`/`..` bileşeni yok, sayı ≤ 2048, yol ≤
  4096 bayt; `OpenFile`'da var olmalı (`directory` ise klasör); `SaveFile` tam bir yol;
  `SaveFiles` `files` ile aynı sayı. Seçim kimlikleri/değerleri sunulanlardan, filtre indisi
  sınır içinde. Geçmeyen sonuç → 2 (log yalnızca neden, yol yok).
- Eşzamanlı en çok 8 seçici; fazlası hemen 2.

### Güvenlik

- **Yalnızca seçilen yollar döner.** Portal uygulamaya dizin listesi, önizleme ya da başka bir yol
  vermez; uygulama kullanıcı seçmeden hiçbir şey öğrenmez.
- **Document portal** arka ucun işi değildir: sandbox'lı (Flatpak) uygulamada xdp ön ucu dönen
  `file://` URI'leri document portal'a kaydedip uygulamaya `/run/user/<uid>/doc/…` yolu verir
  (yalnızca seçilen dosyalar, `SaveFile`'da yazma izniyle). Arka uç gerçek yolu döner; kendisi
  `doc` yolu üretmez. Sandbox'sız uygulama zaten kullanıcı yetkisiyle çalışır.
- Uygulamadan gelen metin (başlık, kabul etiketi, filtre/seçim adları, `current_name`) güvenilmez:
  boyut sınırlı (başlık 256, etiket 128, ad 255 bayt; filtre ≤ 64 × 64 kural; seçim ≤ 32 × 64),
  denetim/yön/sıfır genişlikli karakterler atılır. `parent_window` yalnızca `wayland:<[A-Za-z0-9_-]≤128>`
  kabul edilir.
- **Yol geçişi kapalı (güvenlik bulgusu, 2026-10-16):** `SaveFiles` `files` adları
  eskiden görünümde doğrulanmadan `klasör.join(ad)`'a giriyordu; kum havuzlu uygulama `../x` ya da
  `/abs` adıyla seçilen klasörün dışına yazdırabilirdi. Artık üç katmanda katı denetim — her ad
  boş değil, `/` ve NUL yok, `.`/`..` değil, ≤ 255 bayt (UTF-8 olmayan bayt geçerli):
  (1) portal `options::validate`: tek geçersiz ad bile **bütün isteği** reddeder (sessizce ad
  atlanmaz); (2) seçici `ChooserRequest::from_line` aynı denetimi `files`, `current_name` ve
  `current_folder` (`valid_path`) için yeniden yapar; (3) görünüm `save_files_targets`: çözülen
  ad ve `unique_name` sonucu yeniden denetlenir, her yolun üstü tam olarak seçilen klasördür
  (`parent() == Some(target)`), benzersiz ad tükenirse var olanın üzerine yazmak yerine istek
  iptal edilir. Portal `finish` ayrıca `SaveFiles` hedeflerinin tek klasörde ve adlarının geçerli
  olduğunu denetler. `SaveFile` `current_name` temizlenir (`/` → `_`), `current_file`/
  `current_folder` `valid_path` değilse yok sayılır (yalnız ipucu; yazma hedefi kullanıcının
  seçimi).
- Hiçbir seviyede loglanmaz: yollar, dosya adları, başlangıç klasörü/adı, ebeveyn tanıtıcısı,
  uygulama başlığı. Log yalnızca kip, sayı, yanıt kodu, hata türü.
- **Önizleme yalnız ADR-0007 yalıtımıyla:** resim önizlemesi `nuxut-image-sandbox` işçisinde
  (duvar kağıdı profili: kullanıcı dosyası, her biçim) çözülür, `in_process_fallback = false` —
  işçi yoksa önizleme yok, süreç içinde hiçbir bayt ayrıştırılmaz. MIME türü yalnızca dosya
  ADINDAN (shared-mime-info `globs2`) çıkarılır; içerik koklanmaz (magic yok).
- Sembolik bağ politikası: liste `lstat` ile; bağ kendi adıyla görünür, hedefin türü `stat` ile
  (klasöre bağ gezilebilir, kırık bağ dosya gibi görünür ama açılamaz işaretli). Dönen yol
  kullanıcının gördüğü yoldur (kanonikleştirilmez: uygulama bağın kendisini açar, GTK ile aynı).
- Dizin okuma ana thread dışında, nesil sayacıyla (eski okuma sonucu atılır), en çok 50 000 girdi
  (fazlası "kısaltıldı" uyarısı); her `stat` bloklayabilir (ağ dosya sistemi) → yalnızca işçi
  thread'inde.

### Başlangıç maliyeti ve boşta iş

- Boşta: portal sürecinde yeni thread ya da uyanma yok (zbus kendi yürütücüsünde; çocuk yokken
  iş yok). Seçici açıkken boşta 0 CPU: zamanlayıcı yok, dizin izleme yok (v1; yenile = F5).
- Açılış: çocuk süreç + EGL + font taraması (framework açılış yolu; ilk karede ölçüm
  `NUXUT_PROFILE_STARTUP=1`). Ölçüm sonucu aşağıda "Doğrulama".

## Görünüm

xdg_toplevel pencere, app-id **`nuxut-filechooser`** (Hyprland temel katman kuralı: yüzen, ortalı;
bildirildi 2026-10-15), başlık = uygulamanın verdiği başlık (boşsa "Dosya aç"/"Kaydet"/"Klasör
seç"), başlangıç 960×640, en az 640×420. Ebeveyn: `parent_window` `wayland:<h>` ise ilk configure'dan sonra
`cx.set_window_parent(pencere, h)` (framework abf6864, xdg-foreign içe aktarma); x11/boş tanıtıcıda
ebeveynsiz yüzer. Ebeveyn kapanırsa ilişki sessizce düşer, seçici açık kalır.

```
┌──────────────────────────────────────────────────────────────────────┐
│ [←][↑]  Ev › Belgeler › Rapor            [≣|▦] [👁] [+📁]  [🔍 ara…] │
├──────────────┬───────────────────────────────────────────┬───────────┤
│ Yerler       │  📁 Arşiv                         —  12 Eki│ önizleme  │
│  Ev          │  📄 rapor.pdf                  1,2 MB  3 Eki│ (resim,   │
│  Belgeler    │  🖼 kapak.png                   340 KB  1 Eki│  sandbox) │
│  İndirilenler│  …                                         │           │
│ Aygıtlar     │                                            │           │
│  USB (16 GB) │                                            │           │
│ Yer imleri   │                                            │           │
├──────────────┴───────────────────────────────────────────┴───────────┤
│ Ad: [rapor-son.pdf      ]  [Filtre: PDF ▾] [seçim kutuları]  [Vazgeç] [Kaydet] │
└──────────────────────────────────────────────────────────────────────┘
```

- **Kenar çubuğu** (`List`, gruplu): *Yerler* — Ev, Masaüstü, Belgeler, İndirilenler, Resimler,
  Müzik, Videolar (xdg-user-dirs `user-dirs.dirs`'ten; yoksa ya da dizin yoksa gösterilmez; Ev
  her zaman), *Aygıtlar* — çıkarılabilir birimler (`services::removable_media`, yalnız izleme +
  "Aç" bağlama: bağlı değilse tıklama bağlar, sonra açar; polkit istemi olabilir), *Yer imleri* —
  GTK yer imleri (`$XDG_CONFIG_HOME/gtk-3.0/bookmarks`, salt okunur, ≤ 64 KiB, ≤ 256 girdi, yalnız
  yerel `file://`).
- **Dosya alanı:** liste (`List`, seçim kipi; ad, boyut, değişiklik tarihi) ya da ızgara
  (`FileGrid`, framework c2c2a8b); üst satırdaki görünüm düğmesi ya da Ctrl+1 / Ctrl+2 geçer
  (oturum boyunca; kalıcı tercih yok — 2026-10-16). Seçim iki görünümde de framework
  `Selection` kurallarıyla (Ctrl/Shift tık, Shift/Ctrl ok, Ctrl+Space, Ctrl+A; çoklu yalnız
  `OpenFile` + `multiple`); görünüm değişince ya da içerik yeniden kurulunca (arama, filtre,
  gizli dosyalar) seçimin yalnız ilk öğesi korunur (framework görünümleri küme kurmayı sunmuyor;
  talep aşağıda). Sıra: klasörler önce, doğal sıralama (`dosya2` < `dosya10`), büyük/küçük harf
  duyarsız; gizli dosyalar (`.` ile başlayan) kapalı başlar, Ctrl+H açar.
- **Yol çubuğu:** framework `PathBar` (fw 129899b): ev dizini altındaysa "Ev"den, değilse "Bilgisayar"dan
  (kök) parçalar; taşınca baştan "…". Ctrl+L `begin_edit` ile konum alanı (`~` açılır, göreli yol
  bulunulan klasöre göre, `..` sözcüksel çözülür); verilen yol dosyaysa üst klasöre gidilip seçilir.
- **Arama:** `SearchField`; yazınca bulunulan klasör süzülür (bulanık eşleşme, nucleo; alt klasör
  taranmaz — v1).
- **Önizleme = ızgara küçük resimleri** (2026-10-16): yalnız ızgarada, yalnız görünen
  hücreler (+ iki yanda 12 hücre önden yükleme) için; addan `image/*` olan düzenli dosyalar
  (içerik koklanmaz). `services::file_chooser::thumbs`: tek işçi thread'i, kuyruk her istekte
  değişir (kaydırınca eski istekler düşer, ≤ 256), yeni içerikte süren çözüm iptal edilir. Çözüm
  yalnız `nuxut-image-sandbox` işçisinde (duvar kağıdı profili, `in_process_fallback = false`,
  kova 30/sn, 10 sn duvar saati); işçi yoksa önizleme yok, ikon kalır. Sandbox ilk ızgara
  gösteriminde kurulur (liste kullanan seçicide hiç süreç başlamaz). Çözülemeyen dosya (bozuk,
  sınır) ikonla kalır ve yeniden istenmez. Yan önizleme paneli yok (ızgara yeterli; karar
  2026-10-16).
- **Kaydet:** ad alanı (`TextInput`, `current_name` ile dolu, uzantıdan önceki kısım seçili),
  var olan dosya → `Dialog` "Üzerine yazılsın mı?" (Yıkıcı eylem; Enter etkinleştirmez). Ad bir
  klasörse kaydetmek yerine o klasöre girilir. Geçersiz ad (boş, `/`, `.`/`..`, > 255 bayt, NUL)
  kabul düğmesini kapatır ve satır altında uyarı.
- **Yeni klasör:** düğme → `Dialog` + ad alanı; oluşturulunca listeye gelip seçilir.
- **Boş durum / hata:** `EmptyState` + Fiyu yalnızca boş klasörde (`Mood::Empty`), aramada sonuç
  yokta (`NoResults`) ve okunamayan klasörde (`Error`: izin yok / yok / okuma hatası); maskot
  gizliyse yedek ikon (Folder / Search / Warning), `[appearance] mascot`'a uyar.
- **Erişilebilirlik:** yalnızca framework bileşenleri (odak halkası, klavye gezinmesi; AT-SPI
  köprüsü erişilebilirlik katmanının işi, bileşenlerden gelir).

### Klavye

| Tuş | Eylem |
|---|---|
| Yazdırılabilir karakter (odak listede) | arama alanına geçer ve yazar |
| Enter | listede: klasöre gir / dosyayı seç (kabul); ad alanında: kaydet |
| Esc | arama doluysa temizler; konum alanı açıksa kapatır; değilse vazgeç (1) |
| Ctrl+L | konum alanı (yol yaz, Enter git) |
| Backspace (arama boş, metin alanı değil) / Alt+↑ | üst klasör |
| Alt+← / Alt+→ | geri / ileri |
| Ctrl+H | gizli dosyalar |
| Ctrl+Shift+N | yeni klasör (kaydetme ve klasör seçme kipinde) |
| Ctrl+A (çoklu) | tümünü seç (kenar çubuğundayken dosyalara geçer) |
| Ctrl+1 / Ctrl+2 | liste / ızgara görünümü |
| F5 | yenile |
| Tab / Shift+Tab | kenar çubuğu ↔ liste ↔ ad alanı ↔ düğmeler |
| Ctrl/Shift + tık, Shift+ok, Ctrl+ok + Ctrl+Space | çoklu seçim (`multiple`; ızgarada ←→ ve PgUp/PgDn de) |

### Ekran okuyucu (ADR-0011; atspi benimsemesi 2026-10-16)

Seçici kendi AT-SPI ağacını yayımlar (`src/filechooser_view/access.rs`; düğüm listesi
ARCHITECTURE "Ekran okuyucu"). Odak gerçek klavye odağını izler (odaktaki liste satırı ya da metin
alanı); Orca kullanıcısı yukarıdaki kısayollarla gezer. AT `Click` tıklamayla aynı yoldan gider
(dosya satırını seçer, açmaz — çift tıklama gibi kazara açma olmasın); AT metin yazamaz. Diyalog
açıkken ağaçta yalnız diyalog. Durum satırı (ad hatası, klasör oluşturulamadı) canlı bölge.

**Güvenlik (2026-10-17, HIGH bulgu: portal sandbox'ından kaçış).** a11y bus'ı oturumdaki her
sürece açık (Flatpak'te varsayılan izinli): seçiciyi açtıran sandbox'lı uygulama AT ile satır
seçip "Aç"a basarak kullanıcının seçmediği dosyaya erişebiliyordu (`examples/at_probe` ile
yeniden üretildi: Aç, Kaydet ve üzerine yazma onayı AT'den onaylanıyordu). Kural: **erişim veren
hiçbir eylem AT'den tetiklenemez.** AT `Click` yalnız izin listesindeki gezinme/seçme düğümlerinde
(araç çubuğu — yeni klasör hariç —, yerler, dosya satırları, filtre, seçim kutuları, ad alanını
odaklama, Vazgeç); kabul düğmesi, diyalog düğmeleri ve yeni klasör AT'ye **eylemsiz** (Action
arayüzü yok). Onay yalnız gerçek Wayland girdisiyle (Enter, işaretçi); Orca kullanıcısı odağı
taşır, onayı kendi klavyesi verir. Konum ve ad alanı AT için salt okunur (EditableText yazımı
framework'te atılır; test alanı yeniden okuyarak doğrular). Doğrulama (yalıtılmış ortam,
`framework/tools/a11y-e2e.sh`): düzeltmeden önce üç onay da AT'den gerçekleşti, sonra hiçbiri;
gerçek Enter her iki kipte onayladı.
Çift tık algısı framework `List`/`FileGrid`'e geçince (2026-10-17) AT tıklamalarının çift tık
sayılmaması görünümde korunur: son AT tıklamasından sonra 1 sn (framework çift tık aralığı
400 ms'den uzun) işaretçi çift tıkı satırı açmaz/kabul etmez; çift tıkın bir yarısı AT olabilir.
Gerçek Enter etkilenmez. Doğrulama: `at_probe --gap 50` ile art arda üç AT tıklaması — korumasız
yapı `secret.txt`'yi seçip seçiciyi kapattı (sandbox kaçışı), korumalı yapı açık kaldı, sonuç yok.

## Kurulum ve seçim (portals.conf)

- `nuxut.portal` `Interfaces=org.freedesktop.impl.portal.Settings;org.freedesktop.impl.portal.FileChooser;`.
  Arayüzü listelemek tek başına seçmez: xdp yalnızca `*-portals.conf`'ta adı geçen arka ucu
  kullanır.
- **Seçim kuralı** (2026-10-16; kullanıcı ilkesi "Nuxut bir şeyi yapabiliyorsa
  varsayılan o"; 2026-10-15'teki "açık seçim korunur" kararının yerine): türetilen
  `nuxut-portals.conf`'ta (portal.md "Seçim dosyası") `org.freedesktop.impl.portal.FileChooser` her
  zaman `nuxut;` + kaynaktaki açık FileChooser listesi (yoksa `default` listesi), tekrarsız, `none`
  atılır. Bu makinede `FileChooser = kde` → `nuxut;kde`: Nuxut seçicisi, yedeği kullanıcının
  önceki seçimi. Kaynak yoksa `nuxut;hyprland;gtk`.
- **Yedek:** açık seçim değişiyorsa özgün satır dosyanın sonunda çakışmayan bir grupta saklanır
  (`[X-Nuxut-Backup]` → `org.freedesktop.impl.portal.FileChooser=kde`; xdp yalnız `[preferred]`'ı
  okur). Kullanıcının kendi `hyprland-portals.conf`'una zaten hiç yazılmaz (yalnız okunur).
- **Geri alma (tek komut):** `~/.local/lib/nuxut/xdg-desktop-portal-nuxut --write-portals-conf
  --no-filechooser` (ya da `install.sh --no-filechooser`): dosya seçici devralınmaz, kaynaktaki
  seçim (`kde`) olduğu gibi geçerli. Tercih dosyada `# nuxut-filechooser: off` satırıyla kalır;
  bayraksız yeniden üretim (sonraki install.sh dahil) kapalı tutar. Yeniden açmak:
  `--write-portals-conf --filechooser`. Komutlar dosyanın başlığında yazılı. Etkisi sonraki
  girişte (xdp portals.conf'u açılışta okur).
- Sahiplik: `nuxut-portals.conf` Nuxut'un işaretli dosyasıdır (ilk satır
  `# nuxut-managed: portals`; işaret silinirse kullanıcınındır, hiç yazılmaz). İçerik değişecekse
  install.sh eski Nuxut dosyasını `nuxut-portals.conf.nuxut-yedek-<zaman>` olarak saklar.
- install.sh: `nuxut-filechooser` ikilisi `$LIBEXEC`'e (shell paketiyle derlenir); `--no-filechooser`
  yukarıdaki geri almayı yapar; `--uninstall` ikiliyi siler, seçim dosyası zaten kaldırılır.

## Kararlar (2026-10-15; kullanıcıya sorulmaz)

1. Süreç mimarisi C (yukarıda).
2. ~~Kullanıcının açık FileChooser seçimi korunur~~ → 2026-10-16 (kullanıcı ilkesi):
   Nuxut seçicisi her zaman varsayılan; açık seçim yedek arka uç + `[X-Nuxut-Backup]` yedeği;
   `--no-filechooser` tek komutla geri alır (yukarıda "Kurulum ve seçim").
3. `SaveFiles` çakışmada sormaz, benzersiz ad üretir (`ad (2).uzantı`, 2..=9999; tükenirse 2).
   Gerekçe: çok dosyada dosya başına onay can sıkıcı; spec buna izin veriyor.
4. Filtre glob'ları ASCII büyük/küçük harf duyarsız eşlenir (`*.png` `FOTO.PNG`'yi de gösterir);
   shared-mime-info glob'ları da varsayılan olarak duyarsızdır. Uygulama filtreyi zaten sonra
   kendisi denetler.
5. MIME türü yalnızca addan (globs2 + `subclasses` + `aliases`); içerik koklanmaz. Bilinmeyen tür
   `application/octet-stream`. Klasörler her filtrede görünür (gezinme için).
6. v1'de son klasör hatırlanmaz (uygulama başına iz bırakmamak); `current_folder` yoksa ya da
   erişilemiyorsa Ev.
7. Zaman aşımı yok: kullanıcı seçiciyi istediği kadar açık tutabilir (xdp istek başına bekler,
   diğer istekleri bloklamaz). Yetim kalma stdin kapanmasıyla önlenir.
8. Gizli dosyalar kapalı başlar, tercih kalıcı değil (v1).

## Framework talepleri (iletildi, 2026-10-15; uygulayan)

1. xdg-foreign içe aktarma: `cx.set_window_parent(surface, handle)` — GELDİ (abf6864), kullanılıyor.
2. `FileGrid`: sanal ızgara, ikon/küçük resim + etiket, çoklu seçim, klavye — GELDİ (c2c2a8b), kullanılıyor.
3. `PathBar`: kırıntı yol çubuğu, taşmada katlanma, düzenleme kipi — GELDİ (129899b), kullanılıyor.
4. İkonlar: `ViewList`, `CreateNewFolder`, `Usb` GELDİ (abf6864); `ArrowUpward` GELDİ (a935245), üst klasör
   düğmesinde.
6. `TextInput` aralık seçimi (`select_range`): kaydetme adında uzantıdan önceki gövde seçili başlasın
   — yeni talep; gelene dek imleç sonda.
5. `List` çoklu seçim kipi (`with_selection`) — GELDİ (93bcfd5), kullanılıyor.
7. `List`/`FileGrid` seçim kümesini dışarıdan kurma (`set_selection(iter)`): görünüm değişince ve
   içerik yeniden kurulunca (arama, filtre) çoklu seçim korunsun — yeni talep (2026-10-16); gelene
   dek yalnız ilk öğe korunur.

## Durum ve doğrulama (2026-10-16)

- **Uygulandı:** portal arayüzü + `Request.Close`; `nuxut-filechooser` (yerler, aygıtlar, yer imleri,
  `PathBar`, liste, arama, gizli dosyalar, filtre, seçim kutuları, kaydetme adı + üzerine yazma
  onayı, yeni klasör, `SaveFiles` benzersiz ad, ebeveyn pencere, klavye tablosu, stdin yaşam hattı);
  install.sh adımı ve türetilen seçim dosyasında FileChooser satırı.
- **3/n (2026-10-16):** ızgara görünümü (`FileGrid`), liste/ızgara çoklu seçimi (framework `Selection`),
  yalıtılmış ızgara küçük resimleri (`thumbs`), `ArrowUpward` üst klasör düğmesi. Canlı (izole sway,
  geçici klasör, 14 PNG + bozuk PNG + metin): ızgarada 14 küçük resim işçide çözüldü, bozuk PNG
  ikonla kaldı; Ctrl+A ızgarada hepsini seçti; Ctrl+1 ile listeye dönüş, ↓ + Shift+↓×2 aralık, Enter
  → yalnız 2 düzenli dosya döndü (klasör süzüldü).
- **Henüz yok:** uzantıdan önceki gövdenin seçili başlaması (`TextInput` aralık seçimi talebi),
  görünüm değişiminde çoklu seçimin korunması (talep 7).
- **Testler:** birim (protokol, seçenekler, sonuç doğrulama, çocuk süreç ömrü; dizin okuma, MIME/glob,
  yerler, ad kuralları), özel dbus-daemon entegrasyonu (`portal/tests/file_chooser_dbus.rs`, sahte seçici).
- **Canlı (izole sway, geçici ev dizini; kullanıcı dosyası görünmez):** açma (arama → ↓↓ → Enter →
  `rapor2.pdf`, filtre 0 ve seçim değeri döndü), kaydetme (`yeni-rapor.pdf` + açılır seçim değeri),
  `SaveFiles` (`notlar.txt` → `notlar (2).txt`), üzerine yazma diyaloğu, Ctrl+Shift+N yeni klasör,
  stdin kapanınca iptal (çıkış 1).
- **Uçtan uca (yalıtılmış `dbus-run-session` + izole sway):** gerçek `xdg-desktop-portal-nuxut` +
  gerçek seçici, `busctl` ile `OpenFile` → `0 {"uris": ["file://…/Belgeler/kapak.png"]}`; `SaveFile`
  sürerken `Request.Close` → `1 {}`, seçici süreci kalmadı.
- **Açılış maliyeti** (`NUXUT_PROFILE_STARTUP=1`, debug derleme, llvmpipe/shm, yük altındaki VM): ilk
  kare 1,16 sn (EGL 141 ms, Renderer 207 ms, ilk configure 481 ms); font önbelleği soğukken ilk açılışta
  `TextSystem` beklemesi 5,4 sn ölçüldü (framework font taraması; sharepicker ile ortak). Boşta (seçici
  yokken) portalda ek thread/uyanma yok.
