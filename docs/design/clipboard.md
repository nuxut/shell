# Pano geçmişi (t060)

Durum: aşama 2 uygulandı (framework `DataControl` bağlandı), 2026-10-13. Kararlar: kapsam, gizlilik kuralları, tasarım.
Kaynak: masaüstü boşluk analizi, madde "Pano geçmişi".

## Amaç ve kapsam

Kopyalanan metin ve küçük görsellerin oturum içi geçmişi; listeden seçilen öğe yeniden panoya
konur. v1 kapsamı:

- Varsayılan yalnızca `Selection::Clipboard`; birincil seçim (orta tık) `primary = true` ile
  isteğe bağlı (aşama 2 kararı).
- Metin (UTF-8) ve küçük görsel (`image/png`, `image/jpeg`). Dosya listeleri
  (`text/uri-list`), zengin metin (`text/html`) ayrı öğe olmaz. Bir teklifte metin varsa metin
  saklanır; yalnızca görsel varsa görsel.
- Varsayılan **yalnızca bellek**. Diske yazma isteğe bağlı, varsayılan kapalı.
- Arayüz: başlatıcının "pano" kipi. `msg launcher clipboard` ve kısayolla açılır.

Kapsam dışı (v1): sabitlenmiş öğeler, öğe düzenleme, eşitleme.

## Protokol: framework talebi

Pano izleyicisi odak istemez; `ext-data-control-v1` (yoksa `zwlr-data-control-unstable-v1`)
gerekir. Bu genel bir Wayland yeteneği, shell'e özgü değil, bu yüzden **framework'e aittir**
(`nuxut-wayland`). Shell kendi Wayland bağlantısı açmaz. framework'e talep (2026-10-13):

- `DataControl` (ayrı thread, kendi `wl_display` bağlantısı ya da runtime bağlantısı; framework
  seçer). Yalnızca `Clipboard` seçimi; birincil isteğe bağlı.
- Olay: `SelectionOffered { id, mimes: Vec<String> }` (MIME listesi ≤ 64 öğe, her ad ≤ 256 bayt,
  kırpılmış) ve `SelectionCleared`. Kendi koyduğumuz seçim için `own: bool`.
- `read(id, mime, max_bytes, timeout) -> Result<Vec<u8>, ReadError>`: bloklamayan boru, sınır
  aşılırsa `TooLarge` (okuma durur), süre aşılırsa `Timeout`. Ana thread'de çalışmaz.
- `set_selection(Vec<(mime, Arc<[u8]>)>)`: data-control kaynağıyla seçimi koyar. Odak ya da
  seri numarası istemez; başlatıcı kapandıktan sonra da kaynak sunulmaya devam eder (shell
  yaşadıkça). Yazma `COPY_WRITE_TIMEOUT` ile sınırlı.
- Protokol yoksa `Unsupported`: shell özelliği sessizce kapatır (tek uyarı).

Talep gelene dek shell tarafı bir `ClipboardSource` trait'i arkasında yazılır ve test edilir
(sahte kaynak). Gerçek bağlama framework inince yapılır.

## Gizlilik kuralları

Bir teklif şu durumlarda **okunmaz** ve saklanmaz. Karar MIME listesine bakılarak verilir,
içerik hiç okunmaz:

1. `x-kde-passwordManagerHint` sunuluyor (KeePassXC, KDE). Değeri okunmaz; yalnızca varlığı
   yeter.
2. MIME adında `secret` ya da `password` geçiyor (büyük/küçük harf duyarsız; ör.
   `application/x-nuxut-secret`).
3. Oturum kilitli (`session_lock` etkin) ya da kilit ekranı açık.
4. Kopyalama anındaki etkin pencere sınıfı `[clipboard] exclude_apps` listesinde (varsayılan:
   `org.keepassxc.KeePassXC`, `KeePassXC`, `Bitwarden`, `1Password`). Kaynak, Hyprland'in
   `activewindow` durumundan alınır. Bu bir sezgidir, kesin değildir.
5. Nuxut'un parola alanları (framework `SecretText`) zaten panoya kopyalamaz. Başka
   uygulamaların parola alanı protokolde işaretlenmez: **bilinen sınır**. GTK/Qt parola
   alanları kopyalamayı kendileri engeller; kendi alanından kopyalanan parolayı
   ayıramayız.

Hassas teklif geldiğinde önceki öğeler silinmez. Yalnızca o teklif atlanır.

Tüm içerik kullanıcı verisidir: loglanmaz, `Debug` yalnızca tür ve uzunluk yazar, hata
iletileri içerik taşımaz. Bellekteki öğe silinince (temizle, sınırdan düşme) metin
`zeroize` ile sıfırlanır (nuxut-core zaten bağımlılık).

## Sınırlar

| Sınır | Varsayılan | Aralık |
|---|---|---|
| Öğe sayısı | 100 | 10–500 |
| Metin öğesi | 256 KiB | daha büyüğü saklanmaz |
| Görsel öğesi | 8 MiB (kodlu bayt) | daha büyüğü saklanmaz |
| Toplam bayt | 64 MiB | aşınca en eskiler düşer |
| Okuma süresi | 2 sn | — |
| Teklif başına okuma | 1 (tek MIME) | — |

Aynı içerik (bayt eşit; görselde özet + uzunluk) yeniden kopyalanırsa yeni öğe açılmaz, var olan
öğe başa taşınır. Boş metin ya da yalnızca boşluk saklanmaz.

## Görseller

Görseller kodlu baytlarıyla saklanır; yeniden sunulurken aynı MIME ile verilir. Liste için küçük
resim (en uzun kenar 96 px) `nuxut-image-sandbox` yoluyla **Untrusted** olarak çözülür:
işçi yoksa küçük resim yok, satır yer tutucu ikonla gösterilir. Çözme ana thread'de yapılmaz.

## Diske yazma (isteğe bağlı, varsayılan kapalı)

- `[clipboard] persist = true` ise dosya `$XDG_STATE_HOME/nuxut/clipboard/history.bin` olur,
  izin `0600`, dizin `0700`. Yazım atomik (geçici dosya + rename), 2 sn birleştirmeyle ve
  thread'de.
- Okumada sınırlar yeniden uygulanır (bozuk ya da büyük dosya: tek uyarı, boş geçmiş).
  Biçim sürümlü ve kendi kendini sınırlar (öğe sayısı, uzunluklar başta).
- `persist` kapatılınca ya da "geçmişi temizle" seçilince dosya silinir.
- Bilinen sınır: SSD'de silinen dosyanın blokları kalabilir. Ayarlarda bu not gösterilir.

## Arayüz

- **Başlatıcı kipi** `clipboard`: aynı panel ve satır bileşenleri. Satır: metnin ilk satırı
  (≤ 200 karakter, kontrol karakterleri atılır) ya da küçük resim + boyut; sağda göreli zaman.
  Arama metin öğelerini süzer.
- Enter / tık: öğe panoya konur (`set_selection`), başlatıcı kapanır. Otomatik yapıştırma yok:
  sentetik tuş göndermeyiz.
- Delete: seçili öğeyi siler. Başlıkta "Geçmişi temizle" düğmesi (framework `ConfirmButton`).
- Boş durum: "Pano geçmişi boş". Özellik kapalıysa ya da protokol yoksa açıklayıcı tek satır.
- Kısayol eylemi `clipboard` (`Target::Shell("msg launcher clipboard")`), varsayılan
  `SUPER+SHIFT+V`. `SUPER+V` Hyprland'in varsayılan yapılandırmasında `togglefloating`.

## Yapılandırma (`shell.toml`)

```toml
[clipboard]
enabled = true          # izleyici; false: hiç okuma yok
max_items = 100
images = true           # görsel öğeler
persist = false         # diske yazma
primary = false         # birincil seçim (seç / orta tık) de geçmişe girer
exclude_apps = ["org.keepassxc.KeePassXC", "KeePassXC", "Bitwarden", "1Password"]
```

Şema etiketleri katalogdan (i18n), Ayarlar sayfası Ayarlar'ın.

## IPC

- `msg clipboard list` → JSON v1 `{v, enabled, available, items: [{id, kind, len, preview?, age_s}]}`.
  Önizleme ≤ 200 karakter. Görselin baytı IPC'ye girmez.
- `msg clipboard pick <id>`, `msg clipboard delete <id>`, `msg clipboard clear`.

## Kod düzeni

- `src/services/clipboard/` (UI'dan bağımsız, birim testli):
  - `filter.rs`: gizlilik kararları (saf; MIME listesi + kilit + etkin sınıf → `Accept(mime)` / `Skip(reason)`).
  - `history.rs`: geçmiş modeli (sınırlar, tekilleştirme, zeroize, sıralama).
  - `store.rs`: isteğe bağlı disk biçimi (sürümlü, sınırlı ayrıştırma, atomik yazım).
  - `mod.rs`: `ClipboardSource` trait'i, servis thread'i, olaylar.
- `src/launcher_view.rs` + `src/app/launcher.rs`: kip.
- Ayarlar: `ClipboardConfig::schema`.

## Test planı

- Birim: filtre tablosu (her kural, büyük/küçük harf, sınırlar), geçmiş (tekilleştirme, sıra,
  öğe ve bayt sınırı, en eskinin düşmesi, silme, temizleme), disk biçimi (gidiş-dönüş, bozuk,
  büyük, sürüm), IPC JSON.
- Sahte kaynakla servis: hassas teklif okunmaz (read çağrılmadı kanıtı), kilitliyken okunmaz.
- Canlı (izole sway, ext/wlr data-control): `wl-copy` ile metin ve görsel kopyala, `msg clipboard
  list`, `pick` → `wl-paste` aynı bayt. `wl-copy --type x-kde-passwordManagerHint` ile hassas
  teklif → geçmişte yok. Ekran görüntüsü yalnızca başlatıcı yüzeyi.

## İş bölümü

1. Framework: `nuxut-wayland` data-control (yukarıdaki API).
2. shell işçisi: servis + filtre + geçmiş + disk + IPC + config (sahte kaynakla); ardından
   başlatıcı kipi ve kısayol; framework inince gerçek bağlama ve canlı test.
3. Ayarlar: Ayarlar'da `[clipboard]` bölümü (şemadan).
4. Arch paketi: yok (wl-clipboard yalnızca testte; zaten ekran görüntüsü bağımlılığı).

## Uygulama kararları (aşama 1, 2026-10-13)

Tasarımı netleştiren ya da ondan küçük sapan kararlar. Karar veren: `clipboard` (incelemeye
tabi).

- **Sınırı aşan MIME listesi okunmaz.** Teklif 64'ten çok MIME ya da 256 bayttan uzun bir ad
  taşıyorsa kırpılmaz, tümüyle atlanır (`Malformed`): kırpılan kısım parola ipucunu saklamış
  olabilir. Framework zaten kırpıyorsa bu durum yalnızca savunma katmanıdır.
- **Kilit kaynağı.** "Oturum kilitli" logind `LockedHint`'tir (nuxut-lock bunu ayarlar). logind'e
  bağlanmayan test örneğinde kilit bilinmez (`false`). Okuma sürerken kilitlenirse içerik
  saklanmaz.
- **`enabled = false`** okumayı durdurur ve bellekteki geçmişi bırakır (disk dosyasına
  dokunmaz; o `persist`'e bağlıdır). Yeniden açılınca `persist` açıksa dosyadan yüklenir.
- **Delete tuşu** arama alanıyla paylaşıldığı için yalnızca imleç metnin sonundayken (ileri
  silinecek karakter yokken) seçili öğeyi siler.
- **Göreli zaman ve boyut** satırın alt metnindedir ("5 dk · 1,2 KiB"): framework `ResultItem`
  sağda ek metin yuvası sunmuyor (framework talebi; gelince sağa taşınır).
- **Metin yeniden sunulurken** `text/plain;charset=utf-8`, `text/plain`, `UTF8_STRING`, `STRING`,
  `TEXT` türleriyle verilir (aynı paylaşılan bayt; X11/Xwayland istemcileri de yapıştırabilsin).
- **Sıfırlama sınırı.** İçerik son sahibi bıraktığında sıfırlanır; `set_selection` ile kaynağa
  verilen paylaşılan tampon kaynak bırakana dek sıfırlanmaz (framework talebi: seçim verisini
  bırakırken sıfırlama, yarım okuma tamponunu sıfırlama).
- **Başlatıcı kapalıysa** (`launcher.enabled = false`) pano kipi de açılmaz; IPC (`msg clipboard`)
  çalışmaya devam eder.
- **Aşama 1'de kaynak yoktu** (aşama 2'de bağlandı, aşağıda): framework `DataControl` gelene dek servis başlatılmazdı (tek bilgi
  satırı), `msg clipboard list` `available: false`, başlatıcı "Pano izlenemiyor" satırını gösterir.
- **`exclude_apps` şemada** `custom` türüyle (satır içi TOML dizisi, varsayılan config ile aynı;
  input-v1'in liste türü). Ayarlar'daki düzenleyici Ayarlar'ın.
- **Küçük resim güveni açıkça `Untrusted`** (`thumb_options`; `Limits::ICON`'dan miras alınmaz,
  testle sabit). İşçi yoksa çözülmez, satır yer tutucu ikonla kalır.
- **Disk yazımında artık geçici dosya.** Çökmüş önceki yazımın aynı PID'li `.history.bin.<pid>.tmp`
  artığı yazımdan önce silinir (`create_new` yoksa kalıcı olarak kırılırdı).
- **Canlı görsel test kaynağı** `clipboard-demo` cargo özelliği (varsayılan kapalı, ürüne girmez) +
  `NUXUT_CLIPBOARD_DEMO=1`: birkaç sabit teklifi aralıklı sunar, gerçek panoya dokunmaz. Yalnızca
  DataControl gelene dek başlatıcı kipini izole sway'de görmek için; gerçek bağlama gelince
  kaldırılabilir.

## Uygulama kararları (aşama 2: DataControl bağlaması, 2026-10-13)

Karar veren: `clipboard` (birincil seçimin config ile isteğe bağlı açılması
görev tanımından). Kod: `services/clipboard/source.rs`, `app/clipboard_history.rs`.

- **Adaptör.** `DataControlSource` (`ClipboardSource`) framework `DataControlHandle`'ını tutar.
  Okuma (`handle.read`) ve geri koyma (`handle.set_selection(Selection::Clipboard, …)`)
  servis thread'inden yapılır (ana thread'de framework `MainThread` döner); `cx.data_control_read`
  kullanılmaz. Framework teklif kimliği opaktır: servis kendi `OfferId`'sini görür, eşleme
  yalnızca son 4 teklifi tutar (framework de yalnızca güncel teklifi okur; eskisi `Gone`).
- **İşaretler.** `own`, `sensitive`, `truncated` olduğu gibi servise geçer (`filter::Flags`);
  filtre üçünde de **okumaz** (`Own`, `Sensitive`, `Malformed`). Framework'ün `sensitive`
  kararı, listeden düşen türleri de kapsadığı için shell'in kendi ad kurallarına ek katmandır.
  Geri koyduğumuz öğenin teklifi `own` gelir: geçmişe ikinci kez girmez.
- **`available`.** Servisin anlık görüntüsü artık adaptörün bayrağını yansıtır (paylaşılan
  `AtomicBool` + dürtü; kuyruk dolu olsa da sonraki grupta yayınlanır). Compositor data-control
  sunmuyorsa (`data_control_protocol() == None`) servis hiç başlamaz, `available: false`, tek
  bilgi satırı, panik yok.
- **`enabled = false` izlemeyi durdurur** (`data_control_stop`): compositor tür listesini bile
  göndermez. Yeniden açılınca hemen başlatılır. İstisna (2026-10-16): VM pano köprüsü
  (`x11_bridge`, `docs/design/vm-clipboard.md`) devredeyse yalnız pano izlenir; geçmiş filtresi
  `Disabled` ile hiçbir teklifi okumaz (test: `bridged_selection_passes_every_gate_before_history`).
- **`Stopped` → geri çekilmeli yeniden başlatma** (`Restart`): geçmiş korunur, `available: false`
  yayınlanır; 1, 2, 4 … sn, en çok 5 dk. Son başlangıçtan beri 60 sn çalıştıysa sayaç sıfırlanır
  (ara sıra düşen bağlantı hızla döner; hemen düşen aygıt compositor'ı dövmez). `NoSeat` aynı
  politikayla; `Unsupported` (global kayboldu) yeniden denenmez.
- **Birincil seçim** isteğe bağlı: `[clipboard] primary = false` (varsayılan kapalı; seç-bırak çok
  sık değişir). Açıkken `data_control_watch_primary(true)`; birincil teklifler aynı akışa girer
  (aynı filtre ve sınırlar), seçim geri **her zaman panoya** konur. Birincil seçimin boşalması yok
  sayılır (son pano teklifini gölgelemesin). Bilinen sınır: art arda gelen tekliflerden yalnızca
  sonuncusu okunduğu için aynı anda gelen pano + birincil tekliften yalnızca biri alınır.
- **Sıfırlama.** Framework okuma tamponu `Zeroizing`; geçmiş kendi kopyasını sıfırlar. Geri
  konan paylaşılan `Arc<[u8]>` geçmiş öğesiyle ortaktır: framework son sahip değilse dokunmaz,
  öğe silinip kaynak da bırakılınca son sahip sıfırlar.
- **Canlı doğrulama** (izole sway 1.12, ext-data-control, 2026-10-13): `wl-copy` ile üç metin →
  `msg clipboard list` üç öğe (`available: true`); `wl-copy --type x-kde-passwordManagerHint` ve
  `--type application/x-secret` → `Sensitive`, okunmadı; `msg clipboard pick` → `wl-paste` aynı
  metin, öğe başa geçti, geri gelen teklif `Own` (tekrar yok); başlatıcı pano kipi HEADLESS-1'de.

## Framework talepleri (framework'e iletilir)

1. ~~`nuxut-wayland` `DataControl`~~ — **karşılandı** (framework, 2026-10-13);
   adaptör `services/clipboard/source.rs` (aşama 2).
2. ~~`own` bayrağı tüm kendi seçimlerimizi kapsamalı~~ — **karşılandı** (süreç başına işaret MIME'ı;
   `set_clipboard` dahil). Canlı doğrulandı: geri konan öğe `own` geldi, geçmişe ikinci kez girmedi.
3. ~~Seçim verisini ve yarım okuma tamponunu sıfırlama~~ — **karşılandı** (framework kural 8).
4. `ResultItem` sağda ek metin yuvası (göreli zaman · boyut oraya taşınır). Açık.
