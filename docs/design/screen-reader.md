# Ekran okuyucu: kabuk tarafı

Durum: uygulandı (2026-10-17). İstek: Orca ile bar, paneller, başlatıcı ve bildirimlerde
gezinilebilsin; kilit ekranı aşağıda ("Kilit ekranı"). Framework tarafı ADR-0011
(`../framework/docs/decisions/0011-screen-reader-accesskit.md`); ağaç ve eylem ayrıntıları
`docs/ARCHITECTURE.md` "Ekran okuyucu".

## Kararlar (2026-10-17)

1. **Gezinme kabuğun klavye yoluyla.** Orca yalnız odaktaki düğümü okur; Hyprland Orca'nın
   `KeyboardMonitor` arayüzünü sunmadığından Orca'nın düz inceleme komutları çalışmaz. Bar, OSD ve
   açılır kartlar odak almaz. Bu yüzden kabuk Tab / Shift+Tab / oklar / Enter / boşluk / Escape
   ile kendi gezinmesini verir ve odağı ağaçta bildirir. Tek mekanizma: hedefler son kurulan
   erişilebilirlik ağacının eylemli düğümleri — kart başına ayrı odak kodu yok, yeni kart ağaca
   girdiği an klavyeyle de ulaşılır. Ekran okuyucusuz klavye kullanıcısı da kullanır (odak halkası).
2. **Bar'a giriş `msg bar focus`, kısayol `CTRL+ALT+Tab`.** GNOME'un "sistem denetimlerine geç"
   kısayolu; Orca kullanıcısının alışkanlığı. Hyprland'de yaygın bir varsayılan bağı yok.
   Ayarlar › Kısayollar'dan değiştirilebilir.
3. **Açılan yaprağa odak geçişi.** Bar'da Enter bir yaprak (oturum, hızlı ayarlar, takvim) açtıysa
   odak yaprağın ilk eylemli öğesine geçer: Orca hemen yaprağı okur, kullanıcı Tab'la dolaşır.
4. **Kaydırıcılar 0..=100.** Orca sayısal değeri okur ("64"); 0..=1 aralığında "1 yüzde 100"
   diyordu. Birimli metin değer dizgisinde ("%64", "4000 K", "2,0×").
5. **Açılır bildirim kartının adı başlık + gövde.** Canlı bölgede Orca adı okur, açıklamayı
   değil; yalnız başlık duyuruluyordu. Gövde 160 grafemle sınırlı (ADR-0011 hassas içerik).
6. **OSD canlı bölge.** "Ses %64", "Mikrofon kapalı", "Parlaklık %40", "Büyüteç 2,0×".
7. **Bar değerleri ipuçlarından.** Widget'ın ipucu zaten görünen bilginin özeti; ayrı metin
   kurulmaz, ipucu güncellenince ağaç da güncellenir.
8. **Eylemler mümkünse işaretçi yolundan; AT yalnız gezinir** (aşağıda "Güvenlik"). `Click` düğümün ortasına kabuğun kendi işaretçi
   işleyicisine; bileşenin AT eylemi olan yerlerde (kaydırıcı adımı, bildirim kartı) bileşen
   kendi çevirir. Kullanıcının pencerelerine girdi gitmez.

## Kilit ekranı

**Durum (2026-10-17): ekran okuyucuya açık, AT yalnız okur, kilitliyken hiçbir şey sızdırmaz.**
`nuxut-lock` `lock_session`'dan önce `Context::set_a11y_lock_surfaces(true)` ile framework'ün
kilit yüzeyi adaptörünü açar (ADR-0011 madde 8); kabuğun askısı (`set_a11y_suspended`) kilit
sürecini kapsamaz.

- **Ağaç** (`lock/src/view.rs` `OutputView::access`): kök "Kilit ekranı"; her çıkışta saat ve
  tarih; giriş alanının olduğu çıkışta ayrıca kullanıcı adı, parola alanı ve durum metni (canlı
  bölge). Başka hiçbir şey yok.
- **Parola (bilgi sızıntısı bulgusu, 2026-10-17):** `PasswordInput`, ad "Şifre", açıklama yalnız
  "Boş"/"Dolu". **Değer, uzunluk ve maske ağaçta yok**; metin parçası ve eylem yok → AT-SPI'da
  `Text`, `EditableText`, `Action` arayüzü ve text-changed olayı yok. Düğüm tuş başına değişmez,
  yalnız boş↔dolu geçişinde (uzunluk olay sayısından da çıkarılamaz). `TextInput::access` bilerek
  kullanılmaz: metin parçası accesskit'te `EditableText` açar ve uzunluk maskesini taşır. Bedeli:
  Orca tuş başına geri bildirim vermez; ADR-0011 madde 4'teki uzunluk maskesi kararı kilit
  ekranında geçerli değil.
- **Durum metni** (`access_status`): görsel satırların saldırgana yaramayan alt kümesi —
  "Doğrulanıyor…", "Yanlış şifre" (reddedildi ve çok fazla deneme için aynı), genel hata, Caps
  Lock, sabit "Biraz bekleyip yeniden deneyin", "Parmak izi eşleşmedi". PAM modülü mesajı
  (faillock "hesap N dk kilitli"), parmak izinde kalan deneme ve saniye saniye geri sayım okunmaz.
- **Kilit altındaki masaüstü:** ayrı süreçler. Kabuk, logind `LockedHint` gelince ağacını askıya
  alır (`src/app/idle.rs` → `set_a11y_suspended`): pencere başlıkları, bildirimler, medya ve
  takvim kilitliyken ağaçta yok. Kilit yüzeyleri ekranın tamamını kaplar.
- **Eylemler:** hiçbiri uygulanmaz ve loglanmaz (`lock/src/app.rs` `accessibility_action` boş).
  Odak her zaman gerçek klavye odağıyla parola alanında. Açma düğmesi yok; kilidi açma yalnız
  gerçek Enter ile. **Boş parolada Enter PAM denemesi başlatmaz** (faillock sayacı boşa
  artmasın); parmak izi kendiliğinden çalışır.
- **Günlük:** `RUST_LOG` ne derse desin `nuxut_wayland::a11y`, `accesskit_unix`,
  `accesskit_atspi_common` kapalı (`lock/src/main.rs` `LOG_SILENCED`); parola hiçbir seviyede
  yazılmaz.
- **Doğrulama (2026-10-17):** birim `password_leaks_neither_value_nor_length` (sahte "abc123":
  ağaçta değer, maske ve uzunluk yok; 1 ve 6 karakterde ağaç aynı), `status_hides_attacker_details`;
  canlı `lock/tools/live/09-a11y.sh` (`framework/tools/a11y-e2e.sh` içinde, HOME dahil geçici,
  `RUST_LOG=debug`; `ORCA=1` gerçek Orca, ses çıkmaz). Düşmanca istemci (`examples/at_probe
  --inspect`): parola düğümü yalnız `Accessible | Component`; `Text`, `Action`, `EditableText`
  "Unknown interface"; `DoAction` ve yazma başarısız; doğrulama başlamadı. Orca 51: "Kilit ekranı"
  → "Şifre parola metni" → "Boş" → (abc123 yazılınca bir kez) "Dolu" → Enter → "Doğrulanıyor…" →
  "Yanlış şifre"; text-changed olayı yok, açıklama olayı en çok 2; "abc123", "•" ve uzunluk ne
  Orca çıktısında ne günlükte. Doğru şifre + gerçek Enter açtı.

## Doğrulama (2026-10-17)

- Birim: `screen::access::tests` (bar ağacı adları ve değerleri, adsız etkileşimli düğüm yok;
  `msg bar focus` → özel klavye kipi, Tab/Shift+Tab/sarma, Enter → `Click`, Escape çıkış),
  `quick::volume` (kaydırıcı adı/değeri, `Increment` → `SetVolume`), hızlı anahtarlar.
- AT-SPI dökümü: `framework/tools/a11y-e2e.sh` (yalıtılmış sway + oturum bus'ı + at-spi) içinde
  kabuk ve framework `atspi_dump`: bar, açık panel (oturum, hızlı anahtarlar, ses, ağ, Bluetooth,
  bildirim merkezi), açılır kart, `msg bar focus` sonrası odak, başlatıcı.
- **Gerçek Orca 51:** aynı ortamda, konuşma sunucusuna bağlanamayan adresle
  (`SPEECHD_ADDRESS`; kullanıcının hoparlöründen ses çıkmaz) ve `--debug-file`; tuşlar `wtype`
  ile yalnız test compositor'ına. Orca'nın söyledikleri: "Nuxut logosu düğme" → "Oturum düğme" →
  Enter → "Oturum … panel" → "Kilitle düğme" … "Hızlı anahtarlar panel", "Rahatsız etme aç/kapat
  düğmesi basılı değil, Kapalı" … "Ses düzeyi kaydırıcı …" ; açılır bildirim "Toplantı: Ekip
  toplantısı 10 dakika sonra". Kaydırıcı adımı canlı denenmedi: yalıtılmış ortamda PipeWire
  kullanıcınındır (birim testli).

## Güvenlik: AT eylemi erişim vermez (2026-10-17, HIGH bulgu)

a11y bus'ı oturumdaki her sürece açıktır; Flatpak uygulamalarına da varsayılan olarak izinlidir.
Bu yüzden AT eylemi **kullanıcı niyeti sayılmaz**. Bulgu: dosya seçiciyi açtıran sandbox'lı
uygulama AT `Click` ile satır seçip kabule basarak kullanıcının seçmediği dosyaya erişiyordu
(portal sandbox'ından kaçış); kabukta da AT ile uygulama başlatılabiliyor, pano geçmişi
seçilebiliyor, Bluetooth eşleştirmesi onaylanabiliyordu.

Karar (2026-10-17):

- **AT yalnız okur, odak taşır ve gezinir.** Erişim ya da kalıcı değişiklik veren hiçbir eylem
  AT'den tetiklenemez.
- **Kabuk:** AT `Click`'i benzetilir ama sonuç eylemi yalnız gezinmeyse uygulanır (yaprak
  aç/kapat/sabitle, çalışma alanı, takvim ayı; `app::a11y::assistive_allowed`). Kaydırıcı değeri
  AT'den değişmez. AT `Focus` gezinme odağını taşır.
- **Dosya seçici:** izin listesi; kabul, diyalog onayı ve yeni klasör AT'ye eylemsiz
  (`docs/design/file-chooser.md` "Güvenlik").
- **Gerçek girdi tam yetkili:** işaretçi ve kabuğun klavye gezinmesi (Tab/oklar → Enter; Wayland
  tuşu). Ekran okuyucu kullanıcısı erişilebilirlikten bir şey kaybetmez: Orca odağı taşır,
  etkinleştirmeyi kullanıcının kendi klavyesi verir. Hyprland'de Orca'nın kendi "tıkla"
  komutları zaten çalışmaz.
- **Metin:** AT metin yazamaz (ADR-0011 karar 6); yol ve ad alanları AT için salt okunur.
- **Polkit, ekran paylaşım seçicisi:** bugün ağaç/eylem yayımlamaz. Eklenirse aynı kural (onay,
  parola yalnız gerçek girdi) ve `AT_POLICY:` belgesi zorunlu (bekçi: `tests/it/at_policy.rs`).
- **Kilit ekranı:** ağaç yayımlar, AT yalnız okur ("Kilit ekranı" yukarıda; `AT_POLICY:` belgeli).

Doğrulama: `examples/at_probe` düşmanca AT istemcisi (her düğüme `DoAction`, metin alanlarına
`EditableText` yazımı; alanı yeniden okuyarak). Dosya seçici: düzeltmeden önce Aç, Kaydet ve
üzerine yazma onayı AT'den gerçekleşti; düzeltmeden sonra hiçbiri, gerçek Enter her kipte
onayladı. Kabuk: AT bar'daki Oturum'a basınca panel açıldı (gezinme), Koyu tema anahtarına
basınca hiçbir şey değişmedi; Tab ile anahtara gelip gerçek Enter ayarı değiştirdi.
`app::a11y::tests::assistive_clicks_only_navigate`.
