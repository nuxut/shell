## Hızlı ayarlar: ortak

# Nerede: Ses ve parlaklık kartlarında satır sağındaki yüzde etiketi ile güç kartındaki yalın pil yüzdesi. Anlam: seviyeyi yüzde değeri olarak gösterir.
# Değişkenler: $percent (0–1 arası oran, örnek: 0,64; gösterim katalogda yüzdeye çevrilir).
# [max: 4]
shell-quick-percent-value = { NUMBER($percent, style: "percent", maximumFractionDigits: 0) }

# Nerede: Ağ kartındaki Wi-Fi satırının ve Bluetooth kartındaki anahtar satırının alt metni. Anlam: radyo kapalı.
shell-quick-state-off = Kapalı

# Nerede: Ağ kartında kablolu satırının sağ etiketi ile etkin ağın alt metni, Bluetooth kartında bağlı cihazın alt metni. Anlam: bağlantı etkin.
# [max: 12]
shell-quick-state-connected = Bağlı

# Nerede: Güç kartı pil satırındaki süre gösterimi (format_duration). Anlam: yarım saatten kısa süre yalnızca dakika yazılır (örnek: "35 dk").
# Değişkenler: $minutes (dakika sayısı, örnek: 35).
shell-quick-duration-minutes = { $minutes } dk

# Nerede: Güç kartı pil satırındaki süre gösterimi (format_duration). Anlam: tam saat uzunluğunda süre (örnek: "2 sa").
# Değişkenler: $hours (saat sayısı, örnek: 2).
shell-quick-duration-hours = { $hours } sa

# Nerede: Güç kartı pil satırındaki süre gösterimi (format_duration). Anlam: saat ve dakikalı süre (örnek: "2 sa 5 dk").
# Değişkenler: $hours (saat sayısı, örnek: 2), $minutes (dakika sayısı, örnek: 5).
shell-quick-duration-hours-minutes = { $hours } sa { $minutes } dk

## Parlaklık kartı

# Nerede: Parlaklık kartında birden çok ekran varken kaydırıcının üstündeki satır başlığı. Anlam: cihazın kendi paneli.
shell-quick-brightness-internal-display = Dahili ekran

# Nerede: Parlaklık kartında birden çok ekran varken kaydırıcının üstündeki satır başlığı. Anlam: adı okunamayan harici monitör.
shell-quick-brightness-external-display = Harici ekran

## Güç kartı

# Nerede: Güç kartındaki güç profili seçicinin (segmentli düğme) ilk bölümü. Anlam: güç tasarrufu profili.
# [max: 8]
shell-quick-power-profile-saver = Tasarruf

# Nerede: Güç kartındaki güç profili seçicinin orta bölümü. Anlam: dengeli profil.
# [max: 8]
shell-quick-power-profile-balanced = Dengeli

# Nerede: Güç kartındaki güç profili seçicinin son bölümü. Anlam: en yüksek performans profili.
# [max: 11]
shell-quick-power-profile-performance = Performans

# Nerede: Güç kartındaki pil durumu satırı. Anlam: pil tamamen dolu.
# Değişkenler: $percent (pil düzeyi, 0–1 arası oran, örnek: 1; gösterim katalogda yüzdeye çevrilir).
shell-quick-battery-full = { NUMBER($percent, style: "percent", maximumFractionDigits: 0) } · dolu

# Nerede: Güç kartındaki pil durumu satırı. Anlam: pil şarj oluyor ve dolmasına kalan süre veriliyor.
# Değişkenler: $percent (pil düzeyi, 0–1 arası oran, örnek: 0,4; gösterim katalogda yüzdeye çevrilir), $time (dolma süresi, önceden biçimli, örnek: "35 dk").
shell-quick-battery-charging-time = { NUMBER($percent, style: "percent", maximumFractionDigits: 0) } · şarj oluyor · { $time } içinde dolar

# Nerede: Güç kartındaki pil durumu satırı. Anlam: pil şarj oluyor, süre tahmini yok.
# Değişkenler: $percent (pil düzeyi, 0–1 arası oran, örnek: 0,4; gösterim katalogda yüzdeye çevrilir).
shell-quick-battery-charging = { NUMBER($percent, style: "percent", maximumFractionDigits: 0) } · şarj oluyor

# Nerede: Güç kartındaki pil durumu satırı. Anlam: pil boşalıyor ve kalan süre veriliyor.
# Değişkenler: $percent (pil düzeyi, 0–1 arası oran, örnek: 0,64; gösterim katalogda yüzdeye çevrilir), $time (kalan süre, önceden biçimli, örnek: "2 sa 5 dk").
shell-quick-battery-discharging-time = { NUMBER($percent, style: "percent", maximumFractionDigits: 0) } · { $time } kaldı

# Nerede: Güç kartında profil seçicinin altındaki kısıt uyarısı (ppd nedeni "lap-detected"). Anlam: cihaz kucakta kullanıldığı için performans kısıtlandı.
shell-quick-power-degraded-lap = Performans kısıtlı: kucakta kullanım algılandı

# Nerede: Güç kartında profil seçicinin altındaki kısıt uyarısı (ppd nedeni "high-operating-temperature"). Anlam: sıcaklık nedeniyle performans kısıtlandı.
shell-quick-power-degraded-temperature = Performans kısıtlı: sıcaklık yüksek

# Nerede: Güç kartında profil seçicinin altındaki kısıt uyarısı (bilinmeyen ppd nedeni). Anlam: sistem kaynaklı kısıt.
shell-quick-power-degraded-fallback = Performans kısıtlı: sistem kısıtlıyor

# Nerede: Güç kartındaki pil şarj sınırı anahtar satırının başlığı (yalnızca destekleyen dizüstünde görünür). Anlam: açıkken pil tam dolmaz, ömrü uzar.
# [max: 22]
shell-quick-power-charge-limit = Pil ömrünü koru

# Nerede: Şarj sınırı anahtarının alt metni; açık ya da kapalı, sınırın ne yaptığını söyler. Anlam: şarj bu düzeye gelince durur. Not: "%80'de" gibi ek kullanılmaz (sayıya göre -de/-te/-da değişir), "olunca" eksizdir.
# Değişkenler: $end (bitiş eşiği, 0–1 arası oran, örnek: 0,8; gösterim katalogda yüzdeye çevrilir).
shell-quick-power-charge-limit-at = Şarj { NUMBER($end, style: "percent", maximumFractionDigits: 0) } olunca durur

# Nerede: Şarj sınırı anahtarının alt metni; eşik bilinmiyorsa (aygıt yazılımı belirliyor ya da pillerin eşikleri farklı). Anlam: pil tam dolmadan şarj durur.
shell-quick-power-charge-limit-firmware = Şarj tam dolmadan durur

# Nerede: Şarj sınırı anahtarının alt metni; birden fazla pilde sınır yalnız bazılarında açıksa (anahtar kapalı görünür, açmak hepsini açar).
shell-quick-power-charge-limit-mixed = Pillerin bir kısmında açık

## Ağ kartı

# Nerede: Ağ kartının altındaki uyarı satırı; kurumsal (802.1X) ağa tıklandığında. Anlam: bu ağa panelden bağlanılamaz, Ayarlar uygulaması gerekir.
shell-quick-network-enterprise-notice = Kurumsal ağa Ayarlar'dan bağlanın

# Nerede: Wi-Fi satırının alt metni. Anlam: NetworkManager servisi çalışmıyor, kablosuz yönetilemez.
shell-quick-network-nm-unavailable = NetworkManager çalışmıyor

# Nerede: Wi-Fi satırının alt metni. Anlam: fiziksel donanım anahtarı (uçak modu) kapalı.
shell-quick-network-hardware-off = Donanım anahtarı kapalı

# Nerede: Wi-Fi satırının alt metni. Anlam: etkin kablosuz bağlantı yok.
shell-quick-network-disconnected = Bağlı değil

# Nerede: Ağ kartında ethernet satırının başlığı. Anlam: kablolu bağlantı.
shell-quick-network-wired = Kablolu

# Nerede: Ağ listesinde bağlanmakta olan ağın alt metni. Anlam: bağlantı kuruluyor.
shell-quick-network-connecting = Bağlanıyor…

# Nerede: Ağ listesinde bağlantısı kesilmekte olan ağın alt metni. Anlam: bağlantı kesiliyor.
shell-quick-network-disconnecting = Kesiliyor…

# Nerede: Ağ listesinde kayıtlı ama etkin olmayan ağın alt metni. Anlam: ağ kayıtlı, şu an bağlı değil.
shell-quick-network-saved = Kayıtlı

# Nerede: Ağ kartındaki şifre isteminin metin alanı. Anlam: yer tutucu; girilecek değerin Wi-Fi şifresi olduğu söylenir.
shell-quick-network-password-placeholder = Şifre

# Nerede: Ağ kartındaki şifre isteminin doldurulmuş düğmesi. Anlam: girilen şifreyle bağlanır.
# [max: 8]
shell-quick-network-connect = Bağlan

# Nerede: Ağ kartındaki şifre isteminin met düğmesi. Anlam: istemi kapatır, bağlanmaz.
# [max: 8]
shell-quick-network-cancel = Vazgeç

# Nerede: Şifre isteminin başlığı; yeni şifreli ağa tıklanınca ve NetworkManager şifre isteyince. Anlam: hangi ağın şifresi soruluyor.
# Değişkenler: $ssid (ağ adı, örnek: "ev ağı").
shell-quick-network-password-title = { $ssid } şifresi

# Nerede: Şifre isteminin başlığı; NetworkManager şifreyi reddedip tekrar isterse. Anlam: önceki deneme hatalıydı, şifre yeniden soruluyor.
# Değişkenler: $name (ağ adı, örnek: "ev ağı").
shell-quick-network-password-retry = { $name }: şifre yanlış, tekrar girin

## Bluetooth kartı

# Nerede: Bluetooth anahtar satırının alt metni. Anlam: BlueZ servisi çalışmıyor, Bluetooth yönetilemez.
shell-quick-bluetooth-unavailable = BlueZ çalışmıyor

# Nerede: Bluetooth anahtar satırının alt metni. Anlam: kullanılabilir Bluetooth donanımı (adaptör) yok.
shell-quick-bluetooth-no-adapter = Adaptör yok

# Nerede: Bluetooth anahtar satırının alt metni. Anlam: Bluetooth açık ama bağlı cihaz yok.
shell-quick-bluetooth-no-devices = Bağlı cihaz yok

# Nerede: Bluetooth anahtar satırının alt metni. Anlam: kaç cihazın bağlı olduğu.
# Değişkenler: $count (bağlı cihaz sayısı, örnek: 2).
shell-quick-bluetooth-devices-connected = { $count } cihaz bağlı

# Nerede: Bluetooth cihaz satırının alt metni. Anlam: cihaz bağlı ve pil düzeyi bildiriliyor.
# Değişkenler: $percent (cihaz pil düzeyi, 0–1 arası oran, örnek: 0,8; gösterim katalogda yüzdeye çevrilir).
shell-quick-bluetooth-device-battery = Bağlı · pil { NUMBER($percent, style: "percent", maximumFractionDigits: 0) }

# Nerede: Bluetooth kartının altındaki eşleştirme istemi. Anlam: onay istemi; ekrandaki kodun cihazdakiyle aynı olup olmadığı sorulur.
# Değişkenler: $name (cihaz adı, örnek: "Kulaklık"), $passkey (6 haneli kod, örnek: 000042).
shell-quick-bluetooth-confirm-passkey = { $name }: { $passkey } kodu cihazdakiyle aynı mı?

# Nerede: Bluetooth kartının altındaki eşleştirme istemi. Anlam: cihaz bağlantı kurmak için izin istiyor.
# Değişkenler: $name (cihaz adı, örnek: "Telefon").
shell-quick-bluetooth-authorize-connect = { $name } bağlanmak istiyor

# Nerede: Bluetooth kartının altındaki eşleştirme istemi. Anlam: cihaz bir hizmete erişmek için izin istiyor.
# Değişkenler: $name (cihaz adı, örnek: "Telefon").
shell-quick-bluetooth-authorize-service = { $name } bir hizmete erişmek istiyor

# Nerede: Bluetooth kartının altındaki eşleştirme istemi; PIN/passkey karşı cihazda yazdırılırken. Anlam: kodun karşı cihaza girilmesi isteniyor.
# Değişkenler: $name (cihaz adı, örnek: "Kulaklık"), $code (girilecek kod, örnek: 1234).
shell-quick-bluetooth-display-code = { $name } cihazına girin: { $code }

# Nerede: Bluetooth kartının altındaki eşleştirme istemi; PIN girişi gerektiren eski cihazlar (normalde panele ulaşmaz, servis reddeder). Anlam: cihaz PIN girişi istiyor.
# Değişkenler: $name (cihaz adı, örnek: "Kulaklık").
shell-quick-bluetooth-pin-request = { $name } PIN istiyor

# Nerede: Eşleştirme onay isteminin doldurulmuş düğmesi. Anlam: eşleştirmeyi kabul eder.
# [max: 8]
shell-quick-bluetooth-pair = Eşleştir

# Nerede: Yetkilendirme isteminin doldurulmuş düğmesi. Anlam: isteği kabul eder.
# [max: 8]
shell-quick-bluetooth-allow = İzin ver

# Nerede: Eşleştirme ve yetkilendirme istemlerinin met düğmesi. Anlam: isteği reddeder.
# [max: 8]
shell-quick-bluetooth-deny = Reddet

# Nerede: Hızlı ayarlardaki Bluetooth kartının altındaki hata satırı; PIN/passkey girişi isteyen cihazla eşleştirme isteği geldiğinde (panel isteği otomatik reddeder, system.rs). Anlam: bu cihazla eşleştirme Ayarlar uygulamasından yapılmalı.
shell-quick-bluetooth-pin-rejected = Bu cihaz PIN istiyor; Ayarlar'dan eşleştirin

## Oturum kartı

# Nerede: Oturum kartında geri alınabilir eylem (oturumu kapat, yeniden başlat, kapat) ilk basıldığında düğmenin yerinde açılan uyarı hapı. Anlam: uygulama için ikinci basış bekleniyor.
# Değişkenler: $action (eylem adı, önceden biçimli, örnek: "Yeniden başlat").
shell-quick-session-confirm-armed = { $action } · tekrar bas

# Nerede: Oturum kartında kullanıcı adının altındaki satır; oturum açma adı boşken. Anlam: oturumun açık kalma süresi.
# Değişkenler: $uptime (açık kalma süresi, önceden biçimli, örnek: "2 sa 5 dk").
shell-quick-session-uptime = açık { $uptime }

# Nerede: Oturum kartında kullanıcı adının altındaki satır. Anlam: oturum açma adı ve açık kalma süresi birlikte.
# Değişkenler: $login (oturum açma adı, örnek: "ada"), $uptime (açık kalma süresi, önceden biçimli, örnek: "2 sa 5 dk").
shell-quick-session-detail = { $login } · açık { $uptime }

# Nerede: Oturum kartının altındaki hata satırı. Anlam: eylem uygulanamadı; logind'in hata iletisi gösterilir.
# Değişkenler: $action (eylem adı, önceden biçimli, örnek: "Oturumu kapat"), $message (hata iletisi, önceden biçimli).
shell-quick-session-action-failed = { $action } başarısız: { $message }

## Widget adları

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: çalışma alanları göstergesi widget'ının görünen adı.
shell-bar-widget-workspaces-title = Çalışma alanları

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: saat widget'ının görünen adı.
shell-bar-widget-clock-title = Saat

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: odaklı pencerenin başlığını gösteren widget'ın görünen adı.
shell-bar-widget-active-window-title = Etkin pencere

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: bar'da çalan müziğin kapağını, başlığını ve sanatçısını gösteren widget'ın görünen adı.
shell-bar-widget-now-playing-title = Şimdi çalıyor

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: mikrofon, kamera ya da ekran paylaşımı/kaydı kullanımdayken bar'da beliren göstergenin görünen adı.
shell-bar-widget-privacy-title = Gizlilik göstergesi

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: ekranın kararmasını, kilitlenmesini ve uykuya geçmesini geçici olarak engelleyen bar düğmesinin görünen adı.
shell-bar-widget-caffeine-title = Uyanık tut

# Nerede: Ayarlar uygulamasının widget listesi ve widget ayar sayfaları. Anlam: etkin klavye düzenini gösteren, tıklayınca sıradakine geçen bar göstergesinin görünen adı.
shell-bar-widget-keyboard-layout-title = Klavye düzeni

# Nerede: Ayarlar uygulamasının widget listesi ve widget ayar sayfaları. Anlam: yaklaşan ya da süren takvim etkinliğini bar'da gösteren widget'ın görünen adı.
shell-bar-widget-next-event-title = Sıradaki etkinlik

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: sistem tepsisi widget'ının görünen adı.
shell-bar-widget-tray-title = Sistem tepsisi

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: oturum widget'ının (bar düğmesi ve oturum kartı) görünen adı.
shell-bar-widget-session-title = Oturum

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: bildirimler widget'ının (bar sayacı ve bildirim merkezi) görünen adı.
shell-bar-widget-notifications-title = Bildirimler

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: kenar paneli açan düğme widget'ının görünen adı.
shell-bar-widget-panel-toggle-title = Panel düğmesi

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: ses hızlı ayar kartının görünen adı.
shell-bar-widget-volume-title = Ses

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: parlaklık hızlı ayar kartının görünen adı.
shell-bar-widget-brightness-title = Parlaklık

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: güç hızlı ayar kartının görünen adı.
shell-bar-widget-power-title = Güç

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: ağ hızlı ayar kartının görünen adı.
shell-bar-widget-network-title = Ağ

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: Bluetooth hızlı ayar kartının görünen adı.
shell-bar-widget-bluetooth-title = Bluetooth

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: medya hızlı ayar kartının görünen adı.
shell-bar-widget-media-title = Medya

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: uygulama başlatıcı düğmesinin görünen adı.
shell-bar-widget-launcher-title = Uygulama başlatıcı

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: Nuxut marka işaretini gösteren, tıklayınca uygulama başlatıcısını açan bar düğmesinin görünen adı.
shell-bar-widget-logo-title = Nuxut logosu

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle", kenar panelleri) ve widget ayar sayfaları. Anlam: kenar paneline eklenen, ay takvimini ve seçili günün etkinliklerini gösteren kartın görünen adı.
shell-bar-widget-calendar-title = Takvim

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: sistem izleyici widget'ının görünen adı.
shell-bar-widget-sysmon-title = Sistem izleyici

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: durum göstergeleri widget'ının (ağ, Bluetooth, ses, pil) görünen adı.
shell-bar-widget-status-title = Durum göstergeleri

## Widget ayarları

# Nerede: Oturum, bildirimler ve panel düğmesi widget'larının ayarlarında "panel" seçeneğinin etiketi. Anlam: düğmeye basıldığında hangi kenar panelinin açılacağı seçilir.
shell-bar-widget-opens-panel-label = Açtığı panel

# Nerede: Çalışma alanları widget'ının ayarlarında "count" seçeneğinin etiketi. Anlam: her sayfada görünen workspace (yuva) sayısı.
shell-bar-workspaces-count-label = Sayfa başına workspace

# Nerede: Çalışma alanları widget'ının ayarlarında "style" seçeneğinin etiketi. Anlam: yuvalarda ne çizildiği seçilir (uygulama simgesi, numara, nokta).
shell-bar-workspaces-style-label = Görünüm

# Nerede: Çalışma alanları widget'ının ayarlarında "scroll" seçeneğinin etiketi. Anlam: fare tekerleği/dokunmatik yüzey kaydırmasının hangi workspace'ler arasında geçiş yapacağı seçilir.
shell-bar-workspaces-scroll-label = Kaydırma

# Nerede: Saat widget'ının ayarlarında "format" seçeneğinin etiketi. Anlam: yatay bar saat biçimi (strftime).
shell-bar-clock-format-label = Biçim

# Nerede: Saat widget'ının ayarlarında "vertical_format" seçeneğinin etiketi. Anlam: dikey bar (sol/sağ) saat biçimi; satır sonu karakteri satır böler.
shell-bar-clock-vertical-format-label = Dikey bar biçimi

# Nerede: Saat widget'ının ayarlarında "format" ve "vertical_format" alanlarının altındaki açıklama. Anlam: alan boşsa saat sistemin 12/24 saat tercihine (Bölge ayarları) uyar; doldurulursa yazılan strftime kalıbı kullanılır.
shell-bar-clock-format-describe = Boş bırakılırsa 12/24 saat tercihine uyar; bir biçim kalıbı yazılırsa o kullanılır.

# Nerede: Etkin pencere widget'ının ayarlarında "max_chars" seçeneğinin etiketi. Anlam: pencere başlığının en fazla kaç grafem gösterileceği; uzunsa sonu kırpılır.
shell-bar-active-window-max-chars-label = En fazla karakter

# Nerede: Etkin pencere widget'ının ayarlarında "max_width" seçeneğinin etiketi. Anlam: başlığın piksel cinsinden en geniş hâli; aşarsa kırpılır.
shell-bar-active-window-max-width-label = En geniş hâli

# Nerede: Şimdi çalıyor widget'ının ayarlarında "width" seçeneğinin etiketi. Anlam: widget'ın yatay bar'daki sabit genişliği (piksel).
shell-bar-now-playing-width-label = Genişlik

# Nerede: Şimdi çalıyor widget'ının ayarlarında "width" seçeneğinin altındaki açıklama. Anlam: genişlik parçaya göre değişmez; sığmayan başlık sonunda üç nokta ile kısaltılır.
shell-bar-now-playing-width-describe = Parça değişince bar kaymaz; sığmayan başlık "…" ile kısalır.

# Nerede: Şimdi çalıyor widget'ının ayarlarında "scroll" seçeneğinin etiketi. Anlam: fare tekerleği widget'ın üzerindeyken ne yapılacağı.
shell-bar-now-playing-scroll-label = Tekerlek

# Nerede: Gizlilik göstergesi widget'ının ayarlarında "microphone" anahtarının etiketi. Anlam: bir uygulama mikrofonu kullanırken gösterge görünsün mü.
shell-bar-privacy-microphone-label = Mikrofon

# Nerede: Gizlilik göstergesi widget'ının ayarlarında "camera" anahtarının etiketi. Anlam: bir uygulama kamerayı kullanırken gösterge görünsün mü.
shell-bar-privacy-camera-label = Kamera

# Nerede: Gizlilik göstergesi widget'ının ayarlarında "screen" anahtarının etiketi. Anlam: ekran paylaşılırken ya da kaydedilirken gösterge görünsün mü.
shell-bar-privacy-screen-label = Ekran paylaşımı ve kaydı

# Nerede: Uyanık tut widget'ının ayarlarında "show_when_off" anahtarının etiketi. Anlam: özellik kapalıyken de düğme bar'da görünsün mü (kapalıysa yalnız açıkken görünür).
shell-bar-caffeine-show-when-off-label = Kapalıyken de göster

# Nerede: Sıradaki etkinlik widget'ının ayarlarında "lead_minutes" anahtarının etiketi. Anlam: etkinlik başlamadan kaç dakika önce bar'da görünsün.
shell-bar-next-event-lead-label = Kaç dakika önce görünsün

# Nerede: Sistem tepsisi widget'ının ayarlarında "icon_size" seçeneğinin etiketi. Anlam: tepsi ikonlarının boyu.
shell-bar-tray-icon-size-label = İkon boyu

# Nerede: Bildirimler widget'ının ayarlarında "show_zero" seçeneğinin etiketi. Anlam: okunmamış bildirim yokken bar sayacının da görünmesi.
shell-bar-notifications-show-zero-label = Sıfırken de göster

# Nerede: Panel düğmesi widget'ının ayarlarında "icon" seçeneğinin etiketi. Anlam: düğmede çizilecek ikon seçilir.
shell-bar-panel-toggle-icon-label = İkon

# Nerede: Sistem izleyici widget'ının ayarlarında "cpu" seçeneğinin etiketi. Anlam: bar'da işlemci kullanımının gösterilmesi.
shell-bar-sysmon-cpu-label = İşlemci

# Nerede: Sistem izleyici widget'ının ayarlarında "memory" seçeneğinin etiketi. Anlam: bar'da bellek kullanımının gösterilmesi.
shell-bar-sysmon-memory-label = Bellek

# Nerede: Sistem izleyici widget'ının ayarlarında "gpu" seçeneğinin etiketi. Anlam: bar'da GPU kullanımının gösterilmesi.
shell-bar-sysmon-gpu-label = GPU

# Nerede: Sistem izleyici widget'ının ayarlarında "temperature" seçeneğinin etiketi. Anlam: bar'da sıcaklığın gösterilmesi.
shell-bar-sysmon-temperature-label = Sıcaklık

# Nerede: Sistem izleyici widget'ının ayarlarında "network" seçeneğinin etiketi. Anlam: bar'da ağ hızının gösterilmesi.
shell-bar-sysmon-network-label = Ağ hızı

# Nerede: Sistem izleyici widget'ının ayarlarında "disk" seçeneğinin etiketi. Anlam: bar'da disk doluluğunun gösterilmesi.
shell-bar-sysmon-disk-label = Disk doluluğu

# Nerede: Durum göstergeleri widget'ının ayarlarında "network" seçeneğinin etiketi. Anlam: bar'da ağ göstergesinin görüntülenmesi.
shell-bar-status-network-label = Ağ

# Nerede: Durum göstergeleri widget'ının ayarlarında "bluetooth" seçeneğinin etiketi. Anlam: bar'da Bluetooth göstergesinin görüntülenmesi.
shell-bar-status-bluetooth-label = Bluetooth

# Nerede: Durum göstergeleri widget'ının ayarlarında "volume" seçeneğinin etiketi. Anlam: bar'da ses göstergesinin görüntülenmesi.
shell-bar-status-volume-label = Ses

# Nerede: Durum göstergeleri widget'ının ayarlarında "battery" seçeneğinin etiketi. Anlam: bar'da pil göstergesinin görüntülenmesi.
shell-bar-status-battery-label = Pil

# Nerede: Durum göstergeleri widget'ının ayarlarında "volume_percent" seçeneğinin etiketi. Anlam: ses ikonunun yanında yüzdenin gösterilmesi.
shell-bar-status-volume-percent-label = Ses yüzdesi

# Nerede: Durum göstergeleri widget'ının ayarlarında "battery_percent" seçeneğinin etiketi. Anlam: pil ikonunun yanında yüzdenin gösterilmesi.
shell-bar-status-battery-percent-label = Pil yüzdesi

# Nerede: Oturum, bildirimler ve panel düğmesi widget'larının ayarlarında "panel" seçeneğinin "left" değeri. Anlam: açılacak kenar panelinin sol kenarda olduğu.
shell-schema-panel-left = Sol

# Nerede: Oturum, bildirimler ve panel düğmesi widget'larının ayarlarında "panel" seçeneğinin "right" değeri. Anlam: açılacak kenar panelinin sağ kenarda olduğu.
shell-schema-panel-right = Sağ

# Nerede: Oturum, bildirimler ve panel düğmesi widget'larının ayarlarında "panel" seçeneğinin "top" değeri. Anlam: açılacak kenar panelinin üst kenarda olduğu.
shell-schema-panel-top = Üst

# Nerede: Oturum, bildirimler ve panel düğmesi widget'larının ayarlarında "panel" seçeneğinin "bottom" değeri. Anlam: açılacak kenar panelinin alt kenarda olduğu.
shell-schema-panel-bottom = Alt

# Nerede: Çalışma alanları widget'ının ayarlarında "style" seçeneğinin "icons" değeri. Anlam: dolu yuvalarda uygulama simgesi (bulunamazsa numara), boşta nokta çizildiği.
shell-schema-workspace-style-icons = İkon

# Nerede: Çalışma alanları widget'ının ayarlarında "style" seçeneğinin "numbers" değeri. Anlam: her yuvada workspace numarasının çizildiği.
shell-schema-workspace-style-numbers = Numara

# Nerede: Çalışma alanları widget'ının ayarlarında "style" seçeneğinin "dots" değeri. Anlam: yuvalarda yalnızca nokta çizildiği (dolu büyük, boş küçük).
shell-schema-workspace-style-dots = Nokta

# Nerede: Çalışma alanları widget'ının ayarlarında "scroll" seçeneğinin "all" değeri. Anlam: kaydırmanın boş workspace'ler dahil tüm workspace'ler arasında geçiş yaptığı.
shell-schema-workspace-scroll-all = Tümü

# Nerede: Çalışma alanları widget'ının ayarlarında "scroll" seçeneğinin "occupied" değeri. Anlam: kaydırmanın yalnızca dolu ya da bir monitörde görünen workspace'ler arasında geçiş yaptığı.
shell-schema-workspace-scroll-occupied = Yalnızca dolu

# Nerede: Çalışma alanları widget'ının ayarlarında "scroll" seçeneğinin "off" değeri. Anlam: workspace geçişinin kaydırma ile yapılmadığı.
shell-schema-workspace-scroll-off = Kapalı

# Nerede: Şimdi çalıyor widget'ının ayarlarında "scroll" seçeneğinin "volume" değeri. Anlam: tekerlek sistem ses düzeyini değiştirir.
shell-schema-media-scroll-volume = Ses düzeyi

# Nerede: Şimdi çalıyor widget'ının ayarlarında "scroll" seçeneğinin "track" değeri. Anlam: tekerlek yukarı sonraki, aşağı önceki parçaya geçer.
shell-schema-media-scroll-track = Parça değiştir

# Nerede: Şimdi çalıyor widget'ının ayarlarında "scroll" seçeneğinin "off" değeri. Anlam: tekerlek bu widget üzerinde bir şey yapmaz.
shell-schema-media-scroll-off = Kapalı

## Bar uç ayarları

# Nerede: Ayarlar uygulamasında bar uç eylemlerinin "scroll_step" seçeneğinin etiketi. Anlam: uçta bir kaydırma adımının ses/mikrofon/parlaklıkta değiştirdiği yüzde.
shell-bar-edge-scroll-step-label = Kaydırma adımı

# Nerede: Bar'ın ilk (baş uçtaki) widget'ı için "left_click" seçeneğinin etiketi. Anlam: ilk widget'a sol tıklamada ne yapılacağı seçilir.
shell-bar-edge-start-left-click-label = İlk widget: sol tık

# Nerede: Bar'ın ilk widget'ı için "right_click" seçeneğinin etiketi. Anlam: ilk widget'a sağ tıklamada ne yapılacağı seçilir.
shell-bar-edge-start-right-click-label = İlk widget: sağ tık

# Nerede: Bar'ın ilk widget'ı için "scroll" seçeneğinin etiketi. Anlam: ilk widget üzerinde kaydırma yapıldığında ne olacağı seçilir.
shell-bar-edge-start-scroll-label = İlk widget: kaydırma

# Nerede: Bar'ın son (bitiş uçtaki) widget'ı için "left_click" seçeneğinin etiketi. Anlam: son widget'a sol tıklamada ne yapılacağı seçilir.
shell-bar-edge-end-left-click-label = Son widget: sol tık

# Nerede: Bar'ın son widget'ı için "right_click" seçeneğinin etiketi. Anlam: son widget'a sağ tıklamada ne yapılacağı seçilir.
shell-bar-edge-end-right-click-label = Son widget: sağ tık

# Nerede: Bar'ın son widget'ı için "scroll" seçeneğinin etiketi. Anlam: son widget üzerinde kaydırma yapıldığında ne olacağı seçilir.
shell-bar-edge-end-scroll-label = Son widget: kaydırma

# Nerede: Ayarlar uygulamasında bar uç tık eylemlerinin "widget" seçeneğinin görünen adı. Anlam: tık, uç widget'ın kendi davranışına bırakılır.
shell-schema-edge-click-widget = Widget'ın kendi

# Nerede: Ayarlar uygulamasında bar uç tık eylemlerinin "none" seçeneğinin görünen adı. Anlam: tık hiçbir şey yapmaz; widget'ın davranışı da bastırılır.
shell-schema-edge-click-none = Hiçbiri

# Nerede: Ayarlar uygulamasında bar uç tık eylemlerinin "panel:left" seçeneğinin görünen adı. Anlam: tık sol kenar panelini açar.
shell-schema-edge-click-panel-left = Sol panel

# Nerede: Ayarlar uygulamasında bar uç tık eylemlerinin "panel:right" seçeneğinin görünen adı. Anlam: tık sağ kenar panelini açar.
shell-schema-edge-click-panel-right = Sağ panel

# Nerede: Ayarlar uygulamasında bar uç tık eylemlerinin "panel:top" seçeneğinin görünen adı. Anlam: tık üst kenar panelini açar.
shell-schema-edge-click-panel-top = Üst panel

# Nerede: Ayarlar uygulamasında bar uç tık eylemlerinin "panel:bottom" seçeneğinin görünen adı. Anlam: tık alt kenar panelini açar.
shell-schema-edge-click-panel-bottom = Alt panel

# Nerede: Ayarlar uygulamasında bar uç tık eylemlerinin "panel:clock" seçeneğinin görünen adı. Anlam: tık takvimi (saat popout'unu) açar.
shell-schema-edge-click-panel-clock = Takvim

# Nerede: Ayarlar uygulamasında bar uç tık eylemlerinin "panel:launcher" seçeneğinin görünen adı. Anlam: tık uygulama başlatıcı panelini açar.
shell-schema-edge-click-panel-launcher = Uygulama başlatıcı

# Nerede: Ayarlar uygulamasında bar uç tık eylemlerinin "volume" seçeneğinin görünen adı. Anlam: tık çıkışı sessize alır/açar.
shell-schema-edge-click-volume = Sesi kapat/aç

# Nerede: Ayarlar uygulamasında bar uç tık eylemlerinin "mic" seçeneğinin görünen adı. Anlam: tık mikrofonu sessize alır/açar.
shell-schema-edge-click-mic = Mikrofonu kapat/aç

# Nerede: Ayarlar uygulamasında bar uç kaydırma eylemlerinin "widget" seçeneğinin görünen adı. Anlam: kaydırma, uç widget'ın kendi davranışına bırakılır.
shell-schema-edge-scroll-widget = Widget'ın kendi

# Nerede: Ayarlar uygulamasında bar uç kaydırma eylemlerinin "none" seçeneğinin görünen adı. Anlam: kaydırma hiçbir şey yapmaz; widget'ın davranışı da bastırılır.
shell-schema-edge-scroll-none = Hiçbiri

# Nerede: Ayarlar uygulamasında bar uç kaydırma eylemlerinin "volume" seçeneğinin görünen adı. Anlam: kaydırma ses düzeyini artırır/azaltır.
shell-schema-edge-scroll-volume = Ses

# Nerede: Ayarlar uygulamasında bar uç kaydırma eylemlerinin "mic" seçeneğinin görünen adı. Anlam: kaydırma mikrofonu sessize alır/açar.
shell-schema-edge-scroll-mic = Mikrofon

# Nerede: Ayarlar uygulamasında bar uç kaydırma eylemlerinin "brightness" seçeneğinin görünen adı. Anlam: kaydırma ekran parlaklığını artırır/azaltır.
shell-schema-edge-scroll-brightness = Parlaklık

# Nerede: Ayarlar uygulamasında bar uç kaydırma eylemlerinin "workspace" seçeneğinin görünen adı. Anlam: kaydırma çalışma alanları arasında geçiş yapar.
shell-schema-edge-scroll-workspace = Workspace

## Çalışma alanları

# Nerede: Bar'daki çalışma alanları göstergesinin ayrılmış yuvasının ipucu balonu; bu monitörde special workspace açıkken. Anlam: açık special workspace'in adını bildirir; adlı workspace ipucu da varsa " · " ile birleştirilir.
# Değişkenler: $name (special workspace adı, "special:" öneki atılmış, örnek: "magic").
shell-bar-workspaces-special-tooltip = Özel çalışma alanı: { $name }

## Bildirim kartı zamanı

# Nerede: Bildirim kartı başlığındaki göreli zaman (hem popup yığınında hem merkez panelinde); yeni gelen bildirimlerde. Anlam: bildirim az önce geldi.
# [max: 8]
shell-notifications-time-now = şimdi

# Nerede: Bildirim kartı başlığındaki göreli zaman; bir saat içinde gelen bildirimlerde. Anlam: bildirim kaç dakika önce geldi.
# Değişkenler: $count (dakika sayısı, örnek: 5).
# [max: 4]
shell-notifications-time-minutes = { $count } dk

# Nerede: Bildirim kartı başlığındaki göreli zaman; bir gün içinde gelen bildirimlerde. Anlam: bildirim kaç saat önce geldi.
# Değişkenler: $count (saat sayısı, örnek: 3).
# [max: 4]
shell-notifications-time-hours = { $count } sa

# Nerede: Bildirim kartı başlığındaki göreli zaman; dün gelen bildirimlerde. Anlam: bildirim dün geldi.
# [max: 9]
shell-notifications-time-yesterday = dün

# Nerede: Bildirim kartı başlığındaki göreli zaman; iki günden eski bildirimlerde. Anlam: bildirim kaç gün önce geldi.
# Değişkenler: $count (gün sayısı, örnek: 4).
# [max: 5]
shell-notifications-time-days = { $count } gün

## Bildirim merkezi

# Nerede: Kenar çekmecede bildirim merkezi panelinin başlığı ve Ayarlar uygulamasında notifications.enabled anahtarının satır etiketi. Anlam: bildirim özelliğinin adı.
shell-notifications-title = Bildirimler

# Nerede: Bildirim merkezi panelinde başlık satırının sağındaki düğme. Anlam: geçmişteki tüm bildirimleri siler.
# [max: 16]
shell-notifications-clear-all = Tümünü temizle

# Nerede: Bildirim merkezi panelinde başlığın altındaki anahtarın etiketi ve Ayarlar uygulamasında notifications.dnd anahtarının satır etiketi. Anlam: rahatsız etme kipini açar/kapatır.
# [max: 16]
shell-notifications-dnd-label = Rahatsız etme

# Nerede: Bildirim merkezi panelinde geçmiş boşken büyük ikonun altındaki satır. Anlam: gösterilecek bildirim yok.
shell-notifications-empty = Bildirim yok

# Nerede: Bildirim merkezi panelinde geçmiş grubu başlığı; gönderen uygulamanın adı boş olduğunda. Anlam: uygulaması belli olmayan bildirimlerin grubu.
# [max: 12]
shell-notifications-group-other = Diğer

# Nerede: Bildirim merkezi panelinde geçmiş grubu başlığı; uygulama adı ve bildirim sayısı. Anlam: hangi uygulamanın kaç bildirimi olduğu; sayı yalnızca birden fazlaysa yazılır (çoğul seçici).
# Değişkenler: $name (uygulama adı, örnek: "Sohbet"), $count (bildirim sayısı, örnek: 3).
shell-notifications-group-title =
    { $count ->
        [one] { $name }
       *[other] { $name } · { $count }
    }

## Bildirim ayarları

# Nerede: Ayarlar uygulamasında notifications.popup_timeout_ms anahtarının satır etiketi. Anlam: popup'ın ekranda kalma süresi.
shell-notifications-popup-timeout-label = Bildirim süresi

# Nerede: Ayarlar uygulamasında notifications.max_popups anahtarının satır etiketi. Anlam: aynı anda en fazla kaç popup gösterileceği.
shell-notifications-max-popups-label = En fazla popup

# Nerede: Ayarlar uygulamasında notifications.history_limit anahtarının satır etiketi. Anlam: geçmişte en fazla kaç bildirim tutulacağı.
shell-notifications-history-limit-label = Geçmiş sınırı

## Saat popout'u: takvim

# Nerede: Saat popout'unun altındaki etkinlik listesinde satırın saat sütunu; tüm gün süren etkinliklerde. Anlam: etkinliğin saati yok, tüm gün sürüyor.
# [max: 8]
shell-calendar-all-day = Tüm gün

# Nerede: Saat popout'unun etkinlik listesinin en altındaki satır; seçili günde listede gösterilenden fazla etkinlik varken. Anlam: gösterilemeyen kaç etkinlik daha olduğu.
# Değişkenler: $count (gösterilmeyen etkinlik sayısı, örnek: 3).
shell-calendar-more-events = +{ $count } etkinlik daha

# Nerede: Saat popout'unun etkinlik listesi boşken ortadaki satır; takvim servisi çalışmıyorken. Anlam: etkinlik verisi alınamıyor.
shell-calendar-service-unavailable = Takvim servisi çalışmıyor

# Nerede: Saat popout'unun etkinlik listesi boşken ortadaki satır; seçili gün bugünken. Anlam: bugüne ait etkinlik yok.
shell-calendar-empty-today = Bugün etkinlik yok

# Nerede: Saat popout'unun etkinlik listesi boşken ortadaki satır; seçili gün bugün değilken. Anlam: seçili günde etkinlik yok.
shell-calendar-empty-day = Bu gün etkinlik yok

## Uygulama başlatıcısı

# Nerede: Başlatıcı panelinin en üstündeki arama alanı. Anlam: yer tutucu; alana boşken uygulama arama yazıldığı söylenir.
shell-launcher-search-placeholder = Uygulama ara…

# Nerede: Başlatıcıda sonuç listesi boşken listenin üstündeki satır. Anlam: aramayla eşleşen sonuç yok.
shell-launcher-no-results = Sonuç yok

# Nerede: Başlatıcıda sonuç listesinin ilk satırı; sorgu hesap işlemi olarak değerlendirildiğinde. Anlam: hesap sonucu "= " önekiyle tek satırda gösterilir.
# Değişkenler: $result (hesap sonucu, yerel ayarın ondalık ayracıyla biçimli, örnek: 42).
shell-launcher-calc-result = = { $result }
    .description = Hesap makinesi · Enter sonucu yazar

# Nerede: Başlatıcıda eylemler sayfasının ilk satırı (başlığı uygulama adı olan sayfada). Anlam: uygulamanın varsayılan eylemi, uygulamayı normal biçimde başlatır.
shell-launcher-action-open = Aç

# Nerede: Başlatıcının pano kipinde (SUPER+SHIFT+V) en üstteki arama alanı. Anlam: yer tutucu; yazılan metin pano geçmişindeki metin öğelerini süzer.
shell-launcher-clipboard-placeholder = Pano geçmişinde ara…

# Nerede: Başlatıcının pano kipinde liste boşken ortadaki satır. Anlam: henüz kopyalanmış ve saklanmış bir şey yok.
shell-launcher-clipboard-empty = Pano geçmişi boş

# Nerede: Başlatıcının pano kipinde liste yerine görünen tek satır. Anlam: pano geçmişi ayarlardan kapatılmış (clipboard.enabled).
shell-launcher-clipboard-disabled = Pano geçmişi kapalı

# Nerede: Başlatıcının pano kipinde liste yerine görünen tek satır. Anlam: bileşik sunucu (compositor) panoyu izleme protokolünü sunmuyor; geçmiş tutulamıyor.
shell-launcher-clipboard-unavailable = Pano izlenemiyor: bileşik sunucu desteklemiyor

# Nerede: Başlatıcının pano kipinde arama alanının yanındaki düğme. Anlam: tüm pano geçmişini siler (iki adımlı onay ister).
# [max: 24]
shell-launcher-clipboard-clear = Geçmişi temizle

# Nerede: "Geçmişi temizle" düğmesine bir kez basıldıktan sonra düğmenin etiketi. Anlam: silmek için düğmeye tekrar basılmalı.
# [max: 24]
shell-launcher-clipboard-clear-armed = Temizle · tekrar bas

# Nerede: Başlatıcının pano kipinde görsel öğenin satır başlığı (küçük resmin yanında). Anlam: bu öğe kopyalanmış bir görseldir.
shell-launcher-clipboard-image = Görsel

# Nerede: Başlatıcının pano kipinde her satırın alt metni. Anlam: öğenin ne zaman kopyalandığı ve boyutu.
# Değişkenler: $age (göreli zaman, önceden biçimli, örnek: "5 dk"), $size (boyut, önceden biçimli, örnek: "1,2 KiB").
shell-launcher-clipboard-meta = { $age } · { $size }

## Başlatıcı emoji kipi

# Nerede: Başlatıcının emoji kipinde (`msg launcher emoji`) en üstteki arama alanı. Anlam: yer tutucu; emoji adı ya da anahtar sözcükle (Türkçe veya İngilizce) aranır.
shell-launcher-emoji-placeholder = Emoji ara…

# Nerede: Başlatıcının emoji kipinde, arama boşken ızgaranın ilk bölümünün başlığı. Anlam: kullanıcının en sık/son seçtiği emojiler.
# [max: 32]
shell-launcher-emoji-recent = Sık kullanılanlar

# Nerede: Emoji kipinde her satırın sağındaki küçük metin. Anlam: Unicode emoji grubu "Smileys & Emotion" (yüz ifadeleri, kalpler, duygular).
# [max: 20]
shell-emoji-group-smileys = Yüzler ve duygular

# Nerede: Emoji kipinde satırın sağındaki grup adı. Anlam: Unicode grubu "People & Body" (insanlar, eller, beden).
# [max: 20]
shell-emoji-group-people = İnsanlar ve beden

# Nerede: Emoji kipinde satırın sağındaki grup adı. Anlam: Unicode grubu "Animals & Nature" (hayvanlar, bitkiler).
# [max: 20]
shell-emoji-group-animals = Hayvanlar ve doğa

# Nerede: Emoji kipinde satırın sağındaki grup adı. Anlam: Unicode grubu "Food & Drink".
# [max: 20]
shell-emoji-group-food = Yiyecek ve içecek

# Nerede: Emoji kipinde satırın sağındaki grup adı. Anlam: Unicode grubu "Travel & Places" (taşıtlar, yerler, hava).
# [max: 20]
shell-emoji-group-travel = Seyahat ve yerler

# Nerede: Emoji kipinde satırın sağındaki grup adı. Anlam: Unicode grubu "Activities" (spor, oyun, kutlama).
# [max: 20]
shell-emoji-group-activities = Etkinlikler

# Nerede: Emoji kipinde satırın sağındaki grup adı. Anlam: Unicode grubu "Objects" (nesneler, araçlar, giysi).
# [max: 20]
shell-emoji-group-objects = Nesneler

# Nerede: Emoji kipinde satırın sağındaki grup adı. Anlam: Unicode grubu "Symbols" (işaretler, oklar, burçlar).
# [max: 20]
shell-emoji-group-symbols = Semboller

# Nerede: Emoji kipinde satırın sağındaki grup adı. Anlam: Unicode grubu "Flags" (ülke ve diğer bayraklar).
# [max: 20]
shell-emoji-group-flags = Bayraklar

## Masaüstü ayar eşitlemesi

# Nerede: Hyprland oturumunda ilk eşitlemede, Nuxut satırı Hyprland ayar dosyasına ilk kez eklendiğinde gösterilen bildirimin başlığı (bildirim merkezi ve popup). Anlam: masaüstü ayarlarının (köşe, aralık, gölge, animasyon) yönetimi Nuxut'a devredildi.
shell-hypr-sync-connected-title = Masaüstü ayarların Nuxut'a bağlandı

# Nerede: Bağlantı bildiriminin gövdesi. Anlam: mevcut köşe/aralık/gölge/animasyon ayarlarının Nuxut'unkilerle değiştirildiği; üretilen dosyanın sonuna tek satır eklendiği ve "Geri al" düğmesiyle eski hâline dönülebileceği.
# Değişkenler: $config (üretilen Hyprland ayar dosyasının adı, örnek: "hyprland.lua").
shell-hypr-sync-connected-body = Mevcut köşe, aralık, gölge ve animasyon ayarların Nuxut'unkilerle değiştirildi. { $config } dosyasının sonuna tek bir satır eklendi; "Geri al" ile eski hâline dönebilirsin.

# Nerede: Hyprland ana ayarı Nuxut'a devredildikten sonra (Hoş geldin adımı ya da install.sh --hyprland-takeover) gösterilen bildirimin başlığı. Anlam: kullanıcının hyprland.conf dosyası yedeklendi ve yerine Nuxut'un ana ayarı kondu.
shell-hypr-main-config-title = Hyprland ayarın Nuxut'a geçti

# Nerede: Devralma bildiriminin gövdesi. Anlam: eski ayardan neyin yeni ayara taşındığı (ortam değişkenleri; klavye, fare, imleç, ekran kartı ayarları) ve neyin taşınmadığı (başlangıçta çalışan komutlar, kısayollar, pencere kuralları); eski dosyanın yedekte durduğu ve "Geri al" ile dönülebileceği.
# Değişkenler: $env (taşınan ortam değişkeni sayısı), $settings (taşınan donanım/klavye ayar bloğu sayısı), $exec (taşınmayan başlangıç komutu sayısı), $binds (taşınmayan kısayol sayısı), $rules (taşınmayan pencere kuralı sayısı), $backup (eski dosyanın yedeğinin yolu).
shell-hypr-main-config-body = Taşınan: { $env } ortam değişkeni, { $settings } klavye/fare/donanım ayarı. Taşınmayan: { $exec } başlangıç komutu, { $binds } kısayol, { $rules } pencere kuralı. Eski dosyan { $backup } olarak duruyor; "Geri al" ile dönebilirsin.

# Nerede: Devralma bildiriminin gövdesine eklenen satır, eski ayarda sanal makine pano köprüsü betiği varsa. Anlam: kullanıcının kendi pano köprüsü betiği yeni ayara taşınmadı; sanal makine ile pano paylaşımını artık Nuxut'un köprüsü yapar.
shell-hypr-main-config-clipboard = Kendi pano köprüsü betiğin taşınmadı; sanal makine panosunu artık Nuxut taşıyor.

# Nerede: Bağlantı bildirimindeki eylem düğmesi; `msg hypr undo` komutunu çalıştırır. Anlam: ekleme geri alınır, yedekten önceki ayarlar geri yüklenir.
# [max: 8]
shell-hypr-sync-undo = Geri al

# Nerede: Ayar dili geçişinde (ör. hyprland.conf → hyprland.lua) oturum başına bir kez gösterilen bildirimin başlığı. Anlam: Hyprland ayar dosyasının biçimi değişti.
shell-hypr-sync-switch-title = Masaüstü ayar dosyan değişti

# Nerede: Ayar dili geçiş bildiriminin gövdesi. Anlam: yeni ayar sonraki oturumda etkin olur; düğmeyle hemen geçilebileceği ve geçişte pencerelerle ekranların kısa süreliğine yeniden kurulacağı belirtilir.
# Değişkenler: $config (yeni Hyprland ayar dosyasının adı, örnek: "hyprland.lua").
shell-hypr-sync-switch-body = Hyprland ayarın artık { $config }. Yeni ayar yeniden giriş yapınca geçerli olur; "Şimdi uygula" ile hemen geçebilirsin (pencereler ve ekranlar kısa süre yeniden kurulur).

# Nerede: Ayar dili geçiş bildirimindeki eylem düğmesi; `msg hypr apply-switch` komutunu çalıştırır. Anlam: yeni ayar dili yeniden giriş beklenmeden hemen uygulanır.
# [max: 12]
shell-hypr-sync-apply-switch = Şimdi uygula

# Nerede: `hypr.source` değişikliğinin anlık görüntü/geçmiş kaydındaki etiket; Nuxut satırı kullanıcının Hyprland ayar dosyasına eklendiğinde (src/services/hypr_config/sync.rs). Anlam: Hyprland ayarına, Nuxut'un ürettiği ayar dosyasını yükleyen satırın eklendiği.
shell-hypr-sync-change-added = Hyprland ayarına Nuxut satırı eklendi

# Nerede: `hypr.source` değişikliğinin anlık görüntü/geçmiş kaydındaki etiket; Nuxut'un temel katman satırı kullanıcının Hyprland ayar dosyasının başına eklendiğinde (src/services/hypr_config/sync.rs). Anlam: Hyprland ayarının başına Nuxut'un hazır kısayol ve varsayılanlarını yükleyen satırın eklendiği.
shell-hypr-sync-change-base-added = Hyprland ayarının başına Nuxut temel katmanı eklendi

# Nerede: `hypr.source` değişikliğinin anlık görüntü/geçmiş kaydındaki etiket; satır zaten varken kullanıcının dosyası ilk kez Nuxut tarafından devralınıp yazıldığında (src/services/hypr_config/sync.rs). Anlam: Nuxut ayar dosyasının oluşturulduğu, önceki içeriğin saklandığı.
shell-hypr-sync-change-takeover = Nuxut ayar dosyası (önceki içerik saklandı)

# Nerede: `hypr.source` geri alma değişikliğinin anlık görüntü/geçmiş kaydındaki etiket; "Geri al" ile bağlantı kaldırıldığında (src/services/hypr_config/sync.rs). Anlam: masaüstü ayar bağlantısının (Nuxut satırı ve üretilen dosya) geri alındığı.
shell-hypr-sync-change-undone = Masaüstü ayar bağlantısı geri alındı

# Nerede: `hypr.clipboard_bridge` değişikliğinin anlık görüntü/geçmiş kaydındaki etiket (src/services/clipboard/takeover.rs). Anlam: kullanıcının kendi VM pano köprüsü betiğini başlatan Hyprland satırı, Nuxut köprüsü çalıştığı için yorum satırına çevrildi.
shell-clipboard-bridge-takeover-change = VM pano köprüsü betiği devre dışı (Nuxut köprüsü devraldı)

## Ortak hata metinleri

# Nerede: Ses hızlı ayar kartının altındaki hata satırı; varsayılan cihaz eylemi bilinmeyen bir hedefe verildiğinde ve Bluetooth kartının altındaki hata satırı; cihaz eylemi bilinmeyen bir cihaz yoluna verildiğinde. Anlam: komutun hedef cihazı PipeWire'da/BlueZ'de yok.
shell-error-device-unknown = Bilinmeyen cihaz

## Ses

# Nerede: Ses hızlı ayar kartının altındaki hata satırı; PipeWire bir seviye/sessizlik yazımını reddettiğinde. Anlam: sunucu yazım isteğini hata koduyla geri çevirdi.
# Değişkenler: $code (PipeWire hata kodu, önceden biçimli, örnek: -2).
shell-audio-error-write-rejected = PipeWire yazımı reddetti ({ $code })

# Nerede: Ses hızlı ayar kartının altındaki hata satırı; ses eylemi (seviye/sessizlik) bilinmeyen bir hedefe verildiğinde. Anlam: komutun hedef düğümü PipeWire'da bulunamadı.
shell-audio-error-unknown-target = Bilinmeyen hedef

# Nerede: Ses hızlı ayar kartının altındaki hata satırı; seviye eyleminde. Anlam: düğümün güncel seviyesi sunucudan henüz okunmadı, üzerine yazılamaz.
shell-audio-error-volume-unknown = Seviye henüz bilinmiyor

# Nerede: Ses hızlı ayar kartının altındaki hata satırı; seviye eyleminde. Anlam: hesaplanan hedef seviye geçersiz (sonlu olmayan sayı).
shell-audio-error-volume-invalid = Geçersiz seviye

# Nerede: Ses hızlı ayar kartının altındaki hata satırı; PipeWire'a parametre yazımı gönderilemediğinde. Anlam: seviye/sessizlik parametresi sunucuya iletilemedi.
shell-audio-error-param-write = Parametre yazılamadı

# Nerede: Ses hızlı ayar kartının altındaki hata satırı; varsayılan cihaz eyleminde. Anlam: uygulama akışı varsayılan çıkış/giriş yapılamaz.
shell-audio-error-stream-default = Akış varsayılan yapılamaz

# Nerede: Ses hızlı ayar kartının altındaki hata satırı; varsayılan cihaz eyleminde. Anlam: PipeWire'ın default metadata'sı bulunamadı; oturum yöneticisinin çalışıp çalışmadığı sorulur.
shell-audio-error-no-metadata = PipeWire default metadata'sı yok (oturum yöneticisi çalışıyor mu?)

# Nerede: Ses hızlı ayar kartının altındaki hata satırı; varsayılan cihaz ("default") eyleminde düğüm bulunamadığında (src/services/audio/mod.rs). Anlam: komutun hedef cihazı (düğümü) PipeWire'da bulunamadı.
shell-audio-error-unknown-device = Bilinmeyen cihaz

## Bluetooth

# Nerede: Bluetooth kartının altındaki hata satırı; BlueZ'e gönderilen eylem çağrısı (bağlan, eşleştir, unut, aç/kapa…) D-Bus hatasıyla döndüğünde. Anlam: eylemin başarısızlık nedeni; D-Bus hatasının ham metni gösterilir.
# Değişkenler: $detail (D-Bus hatasının önceden biçimlenmiş metni, örnek: "Connection failed: Host is down").
shell-bluetooth-error-call-failed = Bluetooth işlemi başarısız: { $detail }

# Nerede: Bluetooth kartının altındaki hata satırı; Connect/Pair gibi uzun süren çağrılar süre sınırını aştığında. Anlam: işlem zaman aşımına uğradı.
shell-bluetooth-error-timeout = İşlem zaman aşımına uğradı

# Nerede: Bluetooth kartının altındaki hata satırı; eşleşecek cihaz PIN/passkey girişi istediğinde (kartta PIN girişi yoktur, istek otomatik reddedilir). Anlam: cihaz PIN istediği için eşleştirme kabul edilmedi; Ayarlar uygulamasından eşleştirilmeli.
shell-bluetooth-error-pin-required = Bu cihaz PIN istiyor; Ayarlar'dan eşleştirin

# Nerede: Bluetooth kartının altındaki hata satırı; tarama başlatılırken Bluetooth adaptörü bulunamadığında (src/services/bluetooth/mod.rs). Anlam: tarama yapacak adaptör yok.
shell-bluetooth-error-no-adapter = Adaptör yok

# Nerede: Bluetooth kartının altındaki hata satırı; komut geldiğinde BlueZ servisi bus'ta yokken (src/services/bluetooth/mod.rs). Anlam: Bluetooth yöneticisi (BlueZ) çalışmıyor, komut işlenemez.
shell-bluetooth-error-bluez-unavailable = BlueZ çalışmıyor

# Nerede: Bluetooth kartının altındaki hata satırı; cihaz komutu (bağlan, eşleştir, unut…) bilinmeyen bir cihaza verildiğinde (src/services/bluetooth/mod.rs). Anlam: hedef cihaz listede yok.
shell-bluetooth-error-unknown-device = Bilinmeyen cihaz

# Nerede: Bluetooth kartının altındaki hata satırı; eşleştirme kullanıcı ya da cihaz tarafından reddedildiğinde (src/system.rs). Anlam: eşleştirmenin karşı taraf ya da kullanıcı tarafından reddedildiği.
shell-bluetooth-error-auth-rejected = Eşleştirme reddedildi

# Nerede: Bluetooth kartının altındaki hata satırı; eşleştirme başarısız olduğunda (src/system.rs). Anlam: yanlış PIN/passkey ya da eksik eşleştirme ajanı nedeniyle eşleştirmenin tamamlanamadığı.
shell-bluetooth-error-auth-failed = Eşleştirme başarısız

# Nerede: Bluetooth kartının altındaki hata satırı; eşleştirme iptal edildiğinde (src/system.rs). Anlam: eşleştirme işleminin yarıda kesildiği.
shell-bluetooth-error-auth-canceled = Eşleştirme iptal edildi

# Nerede: Bluetooth kartının altındaki hata satırı; eşleştirme onayı zamanında gelmediğinde (src/system.rs). Anlam: cihazdan onay beklenirken sürenin dolduğu.
shell-bluetooth-error-auth-timeout = Eşleştirme onayı zamanında gelmedi

# Nerede: Bluetooth kartının altındaki hata satırı; cihaza bağlanılamadığında (src/system.rs). Anlam: cihaza ulaşılamadığı; cihaz kapsam dışında ya da kapalı olabilir.
shell-bluetooth-error-connection-attempt-failed = Cihaza ulaşılamadı; kapsam dışı ya da kapalı olabilir

# Nerede: Bluetooth kartının altındaki hata satırı; işlem yapılan öğe zaten varsa, ör. cihaz zaten eşleşmişse (src/system.rs). Anlam: cihazın zaten eşleştirilmiş olduğu.
shell-bluetooth-error-already-exists = Cihaz zaten eşleştirilmiş

# Nerede: Bluetooth kartının altındaki hata satırı; aynı işlem sürerken yeni bir istek gelirse (src/system.rs). Anlam: aynı işlemin hâlen sürdüğü.
shell-bluetooth-error-in-progress = İşlem zaten sürüyor

# Nerede: Bluetooth kartının altındaki hata satırı; adaptör hazır değilken, ör. Bluetooth kapalıyken (src/system.rs). Anlam: adaptörün işleme hazır olmadığı.
shell-bluetooth-error-not-ready = Adaptör hazır değil

# Nerede: Bluetooth kartının altındaki hata satırı; geçersiz bir değer gönderildiğinde, ör. cihaz adı çok uzunsa (src/system.rs). Anlam: gönderilen değerin geçersiz olduğu.
shell-bluetooth-error-invalid-value = Geçersiz değer

# Nerede: Bluetooth kartının altındaki hata satırı; Bluetooth açılmak istendiğinde yazılım engeli (rfkill, ör. uçak kipi) varsa ya da engelin türü okunamadıysa (src/system.rs). Anlam: Bluetooth'un sistemce engellendiği; kullanıcı engeli kaldırabilir.
shell-bluetooth-error-rfkill = Bluetooth engellendi (uçak kipi)

# Nerede: Bluetooth kartının altındaki hata satırı; Bluetooth açılmak istendiğinde donanım anahtarı (rfkill hard) kapalıysa (src/system.rs). Anlam: cihazdaki fiziksel kablosuz anahtarı kapalı; yazılımla açılamaz.
shell-bluetooth-error-rfkill-hard = Bluetooth donanım anahtarıyla kapalı

## Parlaklık

# Nerede: Parlaklık kartının altındaki hata satırı; dahili panelin parlaklığı logind üzerinden yazılamadığında. Anlam: logind SetBrightness çağrısı hata döndürdü; ayrıntı hata metnidir.
# Değişkenler: $detail (logind/D-Bus hatasının önceden biçimlenmiş metni, örnek: "AccessDenied").
shell-brightness-error-logind-set = Parlaklık ayarlanamadı (logind): { $detail }

# Nerede: Parlaklık kartının altındaki hata satırı; parlaklık komutu bilinmeyen bir ekran kimliğine verildiğinde. Anlam: hedef ekran listede yok.
shell-brightness-error-unknown-display = Bilinmeyen ekran

# Nerede: Parlaklık kartının altındaki hata satırı; DDC/CI monitör anlaşılmaz yanıt verdiğinde. Anlam: monitörün cevabı biçim olarak geçersiz.
shell-brightness-error-ddc-bad-reply = Monitör yanıtı anlaşılamadı

# Nerede: Parlaklık kartının altındaki hata satırı; DDC/CI cevabında sağlama toplamı tutmadığında. Anlam: monitörden bozuk yanıt geldi.
shell-brightness-error-ddc-checksum = Monitör yanıtı bozuk geldi

# Nerede: Parlaklık kartının altındaki hata satırı; monitör parlaklık özelliğini desteklemediğinde. Anlam: monitör DDC/CI parlaklık denetimi vermiyor.
shell-brightness-error-ddc-unsupported = Monitör bu özelliği desteklemiyor

# Nerede: Parlaklık kartının altındaki hata satırı; i2c aygıtına erişilemediğinde. Anlam: altta yatan G/Ç hatasının metni.
# Değişkenler: $detail (i2c/Linux hatasının önceden biçimlenmiş metni, örnek: "No such device").
shell-brightness-error-ddc-io = G/Ç hatası: { $detail }

## Ağ

# Nerede: Ağ kartının altındaki hata satırı; NetworkManager'a gönderilen eylem çağrısı (Wi-Fi aç/kapa, tarama, bağlan, etkinleştir…) D-Bus hatasıyla döndüğünde. Anlam: eylemin başarısızlık nedeni; D-Bus hatasının ham metni gösterilir.
# Değişkenler: $detail (D-Bus hatasının önceden biçimlenmiş metni, örnek: "Insufficient privileges").
shell-network-error-call-failed = Ağ işlemi başarısız: { $detail }

# Nerede: Ağ kartının altındaki hata satırı; bağlantı etkinleştirme ya da silme komutu bilinmeyen bir bağlantıya verildiğinde. Anlam: hedef kayıtlı bağlantı NetworkManager'da yok.
shell-network-error-connection-unknown = Bilinmeyen bağlantı

# Nerede: Ağ kartının altındaki hata satırı; etkin olmayan bir bağlantı kapatılmak istendiğinde. Anlam: kapatılacak etkin bağlantı yok.
shell-network-error-connection-inactive = Etkin olmayan bağlantı

# Nerede: Ağ kartının altındaki hata satırı; bilinmeyen bir erişim noktasına bağlanılmak istendiğinde. Anlam: hedef ağ listede yok.
shell-network-error-ap-unknown = Bilinmeyen erişim noktası

# Nerede: Ağ kartının altındaki hata satırı; kurumsal (802.1X) ağa panelden bağlanılmak istendiğinde. Anlam: bu ağlar şifreyle değil, sistem ayarlarından yapılandırılarak bağlanır.
shell-network-error-enterprise = Kurumsal (802.1X) ağlar sistem ayarlarından yapılandırılmalı

# Nerede: Ağ kartının altındaki hata satırı; girilen şifre ağın güvenlik türüne uymadığında. Anlam: şifre uzunluk/biçim koşullarını sağlamıyor.
shell-network-error-invalid-password = Şifre bu ağ için geçersiz

# Nerede: Ağ kartının altındaki hata satırı; Wi-Fi cihazı olmadan kablosuz bağlanılmak istendiğinde. Anlam: sistemde kullanılabilir Wi-Fi aygıtı yok.
shell-network-error-no-wifi-device = Wi-Fi cihazı yok

# Nerede: Ağ kartının altındaki hata satırı; komut geldiğinde NetworkManager servisi bus'ta yokken (src/services/network/mod.rs). Anlam: NetworkManager çalışmıyor, ağ komutları işlenemez.
shell-network-error-nm-unavailable = NetworkManager çalışmıyor

# Nerede: Ağ kartının altındaki hata satırı; gizli ağa bağlanmak için girilen ağ adı 1–32 bayt dışındaysa (src/system.rs). Anlam: girilen ağ adının uzunluk koşulunu sağlamadığı.
shell-network-error-invalid-ssid = Gizli ağ adı 1–32 bayt olmalı

# Nerede: Ağ kartının altındaki hata satırı; işlem bilinmeyen bir ağ cihazına verildiğinde (src/system.rs). Anlam: hedef ağ cihazının NetworkManager'da bulunamadığı.
shell-network-error-device-unknown = Bilinmeyen ağ cihazı

# Nerede: Ağ kartının altındaki hata satırı; NetworkManager işlemi yetki (polkit) gerekçesiyle reddettiğinde (src/system.rs). Anlam: kullanıcının bu ağ ayarını değiştirme izninin olmadığı.
shell-network-error-denied = Bu işlem için izin yok

# Nerede: Ağ kartının/Ayarlar Wi-Fi sayfasının hata satırı; bu bağlantı türünde işlem yapılamadığında, ör. WEP ya da kurumsal ağın şifresi güncellenmek istendiğinde (src/system.rs). Anlam: işlemin bu bağlantı için desteklenmediği.
shell-network-error-unsupported = Bu bağlantı için desteklenmiyor

# Nerede: Ağ kartının/Ayarlar Wi-Fi sayfasının hata satırı; şifresi "saklanmasın, her seferinde sor" olarak ayarlı bir bağlantının şifresi güncellenmek istendiğinde (src/system.rs). Anlam: kullanıcı bu ağın şifresinin saklanmamasını seçmiş; şifre değiştirilmedi.
shell-network-error-secret-not-saved = Bu ağın şifresi saklanmıyor; bağlanırken sorulur

## Güç

# Nerede: Güç kartının altındaki hata satırı; profil değişimi istendiğinde power-profiles-daemon bus'ta yoksa. Anlam: güç profilleri yönetilemiyor.
shell-power-error-daemon-unavailable = power-profiles-daemon kullanılamıyor

# Nerede: Güç kartının altındaki hata satırı; profil yazımı D-Bus hatasıyla döndüğünde. Anlam: profil değiştirilemedi; D-Bus hatasının ham metni gösterilir.
# Değişkenler: $detail (D-Bus hatasının önceden biçimlenmiş metni, örnek: "AccessDenied").
shell-power-error-profile-set = Güç profili değiştirilemedi: { $detail }

# Nerede: Güç kartının altındaki hata satırı; şarj sınırı anahtarına basıldı, polkit izin vermedi (yalnızca etkin yerel oturumda izinli). Anlam: kullanıcının yapabileceği bir şey yoksa nedenini anlat.
shell-power-error-charge-denied = Pil ömrünü koruma değiştirilemedi: izin yok. Yalnızca bilgisayarın başındaki etkin oturumdan değiştirilebilir.

# Nerede: Güç kartının altındaki hata satırı; UPower bu pilin şarj sınırını desteklemediğini söyledi.
shell-power-error-charge-unsupported = Bu pil şarj sınırını desteklemiyor

# Nerede: Güç kartının altındaki hata satırı; şarj sınırı istendiğinde UPower (pil servisi) çalışmıyor ya da güç servisi meşgul.
shell-power-error-charge-unavailable = Pil ayarları şu an kullanılamıyor (UPower çalışmıyor)

# Nerede: Güç kartının altındaki hata satırı; şarj sınırı yazımı başka bir D-Bus hatasıyla döndü.
# Değişkenler: $detail (D-Bus hata adı ya da kısa tür, örnek: "org.freedesktop.UPower.GeneralError", "io").
shell-power-error-charge-failed = Pil ömrünü koruma değiştirilemedi: { $detail }

## Medya

# Nerede: Medya kartının altındaki hata satırı; oynatıcı komutu yüklü olmayan bir oynatıcıya verildiğinde. Anlam: komutun hedef oynatıcısı artık bus'ta yok.
shell-media-error-player-missing = Oynatıcı yok

# Nerede: Medya kartının altındaki hata satırı; konum ayarlanırken parçanın mpris:trackid kimliği yoksa. Anlam: oynatıcı parça kimliği vermediği için konum ayarlanamaz.
shell-media-error-no-track-id = Parçanın kimliği yok (mpris:trackid); konum ayarlanamaz

# Nerede: Medya kartının altındaki hata satırı; oynatıcıya gönderilen komut D-Bus hatasıyla döndüğünde. Anlam: eylemin başarısızlık nedeni; D-Bus hatasının ham metni gösterilir.
# Değişkenler: $detail (D-Bus hatasının önceden biçimlenmiş metni, örnek: "Timeout").
shell-media-error-action-failed = Medya işlemi başarısız: { $detail }

# Nerede: Bar'daki Şimdi çalıyor widget'ının ipucu balonunun son satırı, oynatıcı adından sonra ("Spotify · Çalıyor"). Anlam: parça şu an çalıyor.
# [max: 14]
shell-now-playing-playing = Çalıyor

# Nerede: Bar'daki Şimdi çalıyor widget'ının ipucu balonunun son satırı, oynatıcı adından sonra ("Spotify · Duraklatıldı"). Anlam: parça duraklatılmış.
# [max: 14]
shell-now-playing-paused = Duraklatıldı

# Nerede: Bar'daki gizlilik göstergesinin ipucu balonunda bir satır. Anlam: ekran şu an sürekli yakalanıyor (paylaşım ya da kayıt); ne zaman başladığı ve kaç dakikadır sürdüğü.
# Değişkenler: $time (başlangıç saati, örnek: "14:02"), $minutes (geçen dakika, örnek: 12).
shell-privacy-screen = Ekran yakalanıyor (paylaşım ya da kayıt) · başlangıç { $time }, { $minutes } dk

# Nerede: Bar'daki gizlilik göstergesinin ipucunda, ekran yakalama satırının altında. Anlam: kabuk Hyprland bağlantısını bir süre kaybetti; yakalamanın bittiği doğrulanamadığı için gösterge güvenlik gereği açık tutuluyor.
shell-privacy-screen-uncertain = Bağlantı bir süre koptu; yakalamanın bittiği doğrulanamadı

# Nerede: Bar'daki gizlilik göstergesinin ipucu balonunda bir satır. Anlam: kamerayı kullanan uygulamalar.
# Değişkenler: $apps (virgülle ayrılmış uygulama adları, örnek: "firefox, zoom").
shell-privacy-camera = Kamera kullanımda: { $apps }

# Nerede: Bar'daki gizlilik göstergesinin ipucu balonunda bir satır. Anlam: mikrofonu kullanan uygulamalar.
# Değişkenler: $apps (virgülle ayrılmış uygulama adları, örnek: "Firefox, Discord").
shell-privacy-microphone = Mikrofon kullanımda: { $apps }

# Nerede: Gizlilik göstergesinin ipucunda, adını bildirmeyen uygulama yerine yazılan ad.
shell-privacy-unknown-app = Bilinmeyen uygulama

# Nerede: Bar'daki Uyanık tut düğmesinin ipucu, özellik açıkken. Anlam: ekran kararmıyor/kilitlenmiyor; tıklamak kapatır.
shell-caffeine-on = Uyanık tut açık: ekran kararmaz, kilitlenmez. Kapatmak için tıkla.

# Nerede: Bar'daki Uyanık tut düğmesinin ipucu, özellik kapalıyken. Anlam: tıklamak ekranın kararmasını ve kilitlenmesini engeller.
shell-caffeine-off = Uyanık tut: ekranın kararmasını ve kilitlenmesini engellemek için tıkla.

# Nerede: Bar'daki büyüteç hapının ipucu (yalnız büyütme açıkken görünür). Anlam: büyütme açık; $factor çarpan ("2,0×"); tıklamak kapatır.
shell-bar-magnifier-tooltip = Büyüteç açık: { $factor } · tıklayınca kapanır

# Nerede: Bar'daki klavye düzeni göstergesinin ipucu. Anlam: etkin düzenin tam adı; $name xkb adı ("Turkish", çevrilmez); tıklamak sıradaki düzene geçer.
shell-bar-keyboard-layout-tooltip = Klavye düzeni: { $name } · tıklayınca sıradaki

# Nerede: Bar'daki sıradaki etkinlik hapı. Anlam: $title etkinliği $minutes dakika sonra başlıyor (kısa biçim).
shell-bar-next-event-in = { $title } · { $minutes } dk

# Nerede: Bar'daki sıradaki etkinlik hapı. Anlam: $title etkinliği bir saatten uzakta; $time başlangıç saati.
shell-bar-next-event-at = { $title } · { $time }

# Nerede: Bar'daki sıradaki etkinlik hapı. Anlam: $title etkinliği şu an başlıyor.
shell-bar-next-event-now = { $title } · şimdi

# Nerede: Bar'daki sıradaki etkinlik hapı. Anlam: $title etkinliği sürüyor; $end bitiş saati.
shell-bar-next-event-ongoing = { $title } · bitiş { $end }

# Nerede: Sıradaki etkinlik ipucu. Anlam: $title etkinlik adı, $start–$end saat aralığı; tıklamak takvimi açar.
shell-bar-next-event-tooltip = { $title }
    { $start }–{ $end } · tıklayınca takvim açılır

# Nerede: Sıradaki etkinlik hapı; etkinliğin adı yoksa. Anlam: başlıksız etkinlik.
shell-bar-next-event-untitled = Adsız etkinlik

## Oturum

# Nerede: Oturum kartındaki eylem adları; onay hapında ({ $action } · tekrar bas) ve hata satırında ({ $action } başarısız) değişken olarak geçer. Anlam: kilitleme eyleminin adı.
shell-session-action-lock = Kilitle

# Nerede: Oturum kartındaki eylem adları; onay hapında ({ $action } · tekrar bas) ve hata satırında ({ $action } başarısız) değişken olarak geçer. Anlam: oturumu kapatma eyleminin adı.
shell-session-action-logout = Oturumu kapat

# Nerede: Oturum kartındaki eylem adları; onay hapında ({ $action } · tekrar bas) ve hata satırında ({ $action } başarısız) değişken olarak geçer. Anlam: askıya alma eyleminin adı.
shell-session-action-suspend = Askıya al

# Nerede: Oturum kartındaki eylem adları; onay hapında ({ $action } · tekrar bas) ve hata satırında ({ $action } başarısız) değişken olarak geçer. Anlam: yeniden başlatma eyleminin adı.
shell-session-action-reboot = Yeniden başlat

# Nerede: Oturum kartındaki eylem adları; onay hapında ({ $action } · tekrar bas) ve hata satırında ({ $action } başarısız) değişken olarak geçer. Anlam: sistemi kapatma eyleminin adı.
shell-session-action-poweroff = Kapat

# Nerede: Oturum kartının altındaki hata satırında $message olarak gösterilen ileti; polkit ya da logind eylem için izin vermedi. Anlam: eylem için yetki reddedildi.
shell-session-error-denied = İzin verilmedi

# Nerede: Oturum kartının altındaki hata satırında $message olarak gösterilen ileti; eylem etkileşimli yetki istedi (panel polkit sorusu açmaz). Anlam: yetkilendirme gerekiyor.
shell-session-error-auth-required = Yetki gerekiyor

# Nerede: Oturum kartının altındaki hata satırında $message olarak gösterilen ileti; logind başka bir kapatma/askıya alma işleminin sürdüğünü bildirdi. Anlam: aynı anda tek işlem yapılabilir.
shell-session-error-busy = Başka bir işlem sürüyor

# Nerede: Oturum kartının altındaki hata satırında $message olarak gösterilen ileti; sistem ya da logind bu eylemi desteklemiyor. Anlam: eylem bu sistemde yapılamaz.
shell-session-error-unsupported = Bu sistemde desteklenmiyor

# Nerede: Oturum kartının altındaki hata satırında $message olarak gösterilen ileti; logind geçerli oturum nesnesini bulamadı. Anlam: oturum bulunamadı.
shell-session-error-no-session = Oturum bulunamadı

# Nerede: Oturum kartının altındaki hata satırında $message olarak gösterilen ileti; logind servisine ulaşılamadı (D-Bus bağlantısı yok ya da hizmet yok). Anlam: oturum yöneticisiyle bağlantı kurulamadı.
shell-session-error-unreachable = Oturum yöneticisine ulaşılamadı

# Nerede: Oturum kartının altındaki hata satırında $message olarak gösterilen ileti; logind süre sınırı içinde yanıt vermedi. Anlam: zaman aşımı.
shell-session-error-timeout = Yanıt gelmedi

# Nerede: Oturum kartının altındaki hata satırında $message olarak gösterilen ileti; sınıflandırılamayan bir D-Bus hatası döndü. Anlam: beklenmeyen hata.
shell-session-error-other = Beklenmeyen hata

# Nerede: Oturum kartındaki "açık { $uptime }" süresinin parçası; bir dakikadan kısa oturum süresi. Anlam: süre bir dakikanın altında.
shell-session-uptime-under-minute = 1 dk'dan az

# Nerede: Oturum kartındaki "açık { $uptime }" süresinin parçası; bir günden uzun oturum süresi. Anlam: gün ve saat cinsinden süre.
# Değişkenler: $days (tam gün sayısı, örnek: 3), $hours (kalan tam saat, örnek: 2).
shell-session-uptime-days-hours = { $days } gün { $hours } sa

# Nerede: Oturum kartındaki "açık { $uptime }" süresinin parçası; bir saatten kısa oturum süresi. Anlam: süre yalnızca dakika olarak yazılır.
# Değişkenler: $minutes (dakika sayısı, örnek: 5).
shell-session-uptime-minutes = { $minutes } dk

# Nerede: Oturum kartındaki "açık { $uptime }" süresinin parçası; bir günden kısa, bir saatten uzun oturum süresi. Anlam: saat ve dakika olarak süre.
# Değişkenler: $hours (saat sayısı, örnek: 2), $minutes (dakika sayısı, örnek: 14).
shell-session-uptime-hours-minutes = { $hours } sa { $minutes } dk

## Başlatıcı ayarları

# Nerede: Ayarlar uygulamasında launcher.enabled anahtarının satır etiketi. Anlam: uygulama başlatıcısı özelliğini açar/kapatır.
shell-launcher-enabled-label = Başlatıcı

# Nerede: Ayarlar uygulamasında launcher.calculator anahtarının satır etiketi. Anlam: başlatıcıda ifade yazıldığında hesap sonucu satırının gösterilmesi.
shell-launcher-calculator-label = Hesap makinesi satırı

# Nerede: Ayarlar uygulamasında launcher.systemd_scope anahtarının satır etiketi. Anlam: başlatılan her uygulamanın kendi systemd kapsamında (cgroup) çalıştırılması.
shell-launcher-systemd-scope-label = Uygulamaları ayrı systemd kapsamında başlat

# Nerede: Ayarlar uygulamasında launcher.width anahtarının satır etiketi; değere px birimi eklenir. Anlam: başlatıcı panelinin piksel cinsinden genişliği.
shell-launcher-width-label = Genişlik

# Nerede: Ayarlar uygulamasında launcher.rows anahtarının satır etiketi. Anlam: başlatıcıda aynı anda görünen sonuç satırı sayısı.
shell-launcher-rows-label = Görünen satır

## Masaüstü ayar eşitlemesi

# Nerede: Ayarlar uygulamasında compositor.ensure_source anahtarının satır etiketi. Anlam: Nuxut'un ayar dosyasının kullanıcının Hyprland ayar dosyasına bağlanması (source satırı eklenmesi).
shell-hypr-ensure-source-label = Masaüstü ayar dosyasına bağlan

# Nerede: Ayarlar uygulamasında compositor.sync_look anahtarının satır etiketi. Anlam: köşe, gölge ve bulanıklık ayarlarının pencerelere uygulanması.
shell-hypr-sync-look-label = Köşe, gölge ve bulanıklığı pencerelere uygula

# Nerede: Ayarlar uygulamasında compositor.sync_motion anahtarının satır etiketi. Anlam: hareket (animasyon) ayarlarının pencerelere uygulanması.
shell-hypr-sync-motion-label = Hareketi pencerelere uygula

# Nerede: Ayarlar uygulamasında compositor.sync_windows anahtarının satır etiketi. Anlam: pencere aralığı ve kenarlığının uygulanması.
shell-hypr-sync-windows-label = Pencere aralığı ve kenarlığını uygula

# Nerede: Ayarlar uygulamasında compositor.sync_keybinds anahtarının satır etiketi. Anlam: kısayolların Hyprland'e uygulanması.
shell-hypr-sync-keybinds-label = Kısayolları uygula

# Nerede: Ayarlar uygulamasında windows.gap_inner anahtarının satır etiketi; değere px birimi eklenir. Anlam: pencereler arasındaki boşluk.
shell-hypr-gaps-inner-label = Pencereler arası boşluk

# Nerede: Ayarlar uygulamasında windows.gap_outer anahtarının satır etiketi; değere px birimi eklenir. Anlam: ekran kenarı ile pencereler arasındaki boşluk.
shell-hypr-gaps-outer-label = Kenar boşluğu

# Nerede: Ayarlar uygulamasında windows.border anahtarının satır etiketi; değere px birimi eklenir. Anlam: pencere kenarlığının kalınlığı.
shell-hypr-border-size-label = Kenarlık kalınlığı

# Nerede: Ayarlar uygulamasında windows.blur anahtarının satır etiketi. Anlam: pencere arka planının bulanıklaştırılması.
shell-hypr-blur-label = Bulanıklık

# Nerede: Ayarlar uygulamasında windows.blur_size anahtarının satır etiketi; değere px birimi eklenir. Anlam: bulanıklığın şiddeti.
shell-hypr-blur-size-label = Bulanıklık miktarı

# Nerede: Üretilen nuxut.lua dosyasının sonundaki hata bildirimi; Hyprland dosyayı yükleyemezse hl.notification.create ile ekranda gösterilir. Anlam: Nuxut'un masaüstü ayarları uygulanamadı; nedeni Lua hatasıdır.
# Değişkenler: $detail (Lua hatasının önceden biçimlenmiş metni, örnek: "[string \"nuxut.lua\"]:12: attempt to call a nil value").
shell-hypr-lua-apply-error = Nuxut ayarları uygulanamadı: { $detail }

## Ekranlar

# Nerede: Ayarlar uygulamasında displays.confirm_seconds anahtarının satır etiketi; değere sn birimi eklenir. Anlam: riskli ekran değişikliğinden sonra onay penceresinin açık kalacağı süre.
shell-displays-confirm-seconds-label = Ekran değişikliğini onaylama süresi

# Nerede: Üretilen nuxut/displays.lua dosyasının sonundaki hata bildirimi; Hyprland modülü yükleyemezse hl.notification.create ile ekranda gösterilir. Anlam: ekran ayarları uygulanamadı; nedeni Lua hatasıdır.
# Değişkenler: $detail (Lua hatasının önceden biçimlenmiş metni, örnek: "[string \"displays.lua\"]:8: attempt to index a nil value").
shell-displays-lua-apply-error = Nuxut ekran ayarları uygulanamadı: { $detail }

# Nerede: Ayarlar Ekranlar sayfasının hata/ayrıntı satırı; displays servisi Hyprland'e yazma erişimi olmadan komut göndermek zorunda kaldığında. Anlam: shell'in Hyprland'e yazma izni yok.
shell-displays-error-no-write-access = Hyprland'e yazma erişimi yok

# Nerede: Ayarlar Ekranlar sayfasının hata/ayrıntı satırı; apply isteği ekran komutu göndermeyen bir shell örneğine düştüğünde. Anlam: bu shell örneği Hyprland'e ekran komutu göndermez.
shell-displays-error-no-display-commands = bu örnek Hyprland'e ekran komutu göndermez

# Nerede: Ayarlar Ekranlar sayfasının hata/ayrıntı satırı; Hyprland yapılandırması okunamadığında. Anlam: Hyprland'in ayar dili (yapılandırma sözdizimi) okunamadı.
shell-displays-error-language-unreadable = Hyprland ayar dili okunamadı

# Nerede: Ayarlar Ekranlar sayfasının hata/ayrıntı satırı; onay süresi dolup eski ayarlara dönülürken geri dönüş Hyprland'e gönderilemediğinde. Anlam: geri dönüş gönderilemedi; Hyprland tarafındaki bekçi (watchdog) geri dönecek.
shell-displays-error-revert-send-failed = geri dönüş Hyprland'e gönderilemedi; Hyprland tarafındaki bekçi geri dönecek

# Nerede: Ayarlar Ekranlar sayfasının hata/ayrıntı satırı; shell yeniden başladığında onaylanmamış ekran değişikliği geri dönülürken. Anlam: shell yeniden başladı, değişiklik geri alındı.
shell-displays-error-shell-restarted = shell yeniden başladı

## Kısayollar

# Nerede: Ayarlar uygulamasında keybinds.volume_up anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: sesi yükseltme eylemi.
shell-keybinds-volume-up-label = Sesi aç

# Nerede: Ayarlar uygulamasında keybinds.volume_down anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: sesi kısma eylemi.
shell-keybinds-volume-down-label = Sesi kıs

# Nerede: Ayarlar uygulamasında keybinds.volume_mute anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: sesi sessize alma eylemi.
shell-keybinds-volume-mute-label = Sesi kapat

# Nerede: Ayarlar uygulamasında keybinds.mic_mute anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: mikrofonu sessize alma eylemi.
shell-keybinds-mic-mute-label = Mikrofonu kapat

# Nerede: Ayarlar uygulamasında keybinds.brightness_up anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: ekran parlaklığını artırma eylemi.
shell-keybinds-brightness-up-label = Parlaklığı artır

# Nerede: Ayarlar uygulamasında keybinds.brightness_down anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: ekran parlaklığını azaltma eylemi.
shell-keybinds-brightness-down-label = Parlaklığı azalt

# Nerede: Ayarlar uygulamasında keybinds.media_play anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: oynatmayı başlatma/duraklatma eylemi.
shell-keybinds-media-play-label = Oynat / duraklat
# Nerede: Ayarlar uygulamasında keybinds.media_pause anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: klavyedeki ayrı duraklat tuşu; oynatmayı duraklatır ya da sürdürür.
shell-keybinds-media-pause-label = Duraklat tuşu

# Nerede: Ayarlar uygulamasında keybinds.media_next anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: sonraki parçaya geçme eylemi.
shell-keybinds-media-next-label = Sonraki parça

# Nerede: Ayarlar uygulamasında keybinds.media_prev anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: önceki parçaya geçme eylemi.
shell-keybinds-media-prev-label = Önceki parça

# Nerede: Ayarlar uygulamasında keybinds.panel_right anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: sağ kenar panelini açıp kapatan eylem.
shell-keybinds-panel-right-label = Sağ paneli aç/kapat

# Nerede: Ayarlar uygulamasında keybinds.panel_left anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: sol kenar panelini açıp kapatan eylem.
shell-keybinds-panel-left-label = Sol paneli aç/kapat

# Nerede: Ayarlar uygulamasında keybinds.panel_clock anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: takvimi (saat popout'unu) açıp kapatan eylem.
shell-keybinds-panel-clock-label = Takvimi aç/kapat

# Nerede: Ayarlar uygulamasında keybinds.bar_toggle anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: barı gösterip gizleyen eylem.
shell-keybinds-bar-toggle-label = Barı göster/gizle

# Nerede: Ayarlar uygulamasında keybinds.bar_focus anahtarının satır etiketi. Anlam: klavyeyle (ve ekran okuyucuyla) bar'da gezinmeye başlama eylemi.
shell-keybinds-bar-focus-label = Bar'da klavyeyle gezin

# Nerede: Ayarlar uygulamasında keybinds.shot_region anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: bölge seçerek ekran görüntüsü alma eylemi.
shell-keybinds-shot-region-label = Ekran görüntüsü: bölge

# Nerede: Ayarlar uygulamasında keybinds.shot_output anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: tüm ekranın ekran görüntüsünü alma eylemi.
shell-keybinds-shot-output-label = Ekran görüntüsü: ekran

# Nerede: Ayarlar uygulamasında keybinds.shot_window anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: etkin pencerenin ekran görüntüsünü alma eylemi.
shell-keybinds-shot-window-label = Ekran görüntüsü: pencere

# Nerede: Ayarlar uygulamasında keybinds.clipboard anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: başlatıcıyı pano geçmişi kipinde açma eylemi.
shell-keybinds-clipboard-label = Pano geçmişi

# Nerede: Ayarlar uygulamasında keybinds.emoji anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: başlatıcıyı emoji seçici kipinde açma eylemi.
shell-keybinds-emoji-label = Emoji seçici

## İlk açılış

# Nerede: Ayarlar uygulamasında setup.welcome anahtarının satır etiketi. Anlam: ilk açılışta Hoş geldin penceresinin gösterilmesi.
shell-setup-welcome-label = İlk açılışta Hoş geldin penceresi

## Ayar etiketleri

# Nerede: Ayarlar uygulamasının genel ayar listesinde `frame.enabled` anahtarının etiketi. Anlam: ekran kenarlarındaki çerçevenin açılıp kapatıldığı seçilir.
shell-config-frame-enabled-label = Çerçeve

# Nerede: Ayarlar uygulamasında `frame.thickness` anahtarının etiketi. Anlam: çerçeve kenarlarının kalınlığı (piksel) ayarlanır.
shell-config-frame-thickness-label = Çerçeve kalınlığı

# Nerede: Ayarlar uygulamasında `bar.edge` anahtarının etiketi. Anlam: barın hangi ekran kenarına yerleşeceği seçilir.
shell-config-bar-edge-label = Bar kenarı

# Nerede: Ayarlar uygulamasında `bar.size` anahtarının etiketi. Anlam: barın kalınlığı (piksel) ayarlanır.
shell-config-bar-size-label = Bar kalınlığı

# Nerede: Ayarlar uygulamasında `bar.padding` anahtarının etiketi. Anlam: bar içeriğinin uçlardaki iç boşluğu (piksel) ayarlanır.
shell-config-bar-padding-label = Bar iç boşluğu

# Nerede: Ayarlar uygulamasında `bar.spacing` anahtarının etiketi. Anlam: bar üzerindeki widget'lar arasındaki boşluk (piksel) ayarlanır.
shell-config-bar-spacing-label = Widget aralığı

# Nerede: Ayarlar uygulamasında `panels.<kenar>.enabled` anahtarının etiketi (dört kenar için tek metin). Anlam: o kenardaki kenar panelinin açılıp kapatıldığı seçilir.
shell-config-panels-enabled-label = Panel

# Nerede: Ayarlar uygulamasında `panels.<kenar>.size` anahtarının etiketi (dört kenar için tek metin). Anlam: panelin kenardan içeri derinliği (piksel) ayarlanır.
shell-config-panels-size-label = Panel derinliği

# Nerede: Ayarlar uygulamasında `media.remote_art` anahtarının etiketi. Anlam: çalan parçanın kapağının ağdan indirilip indirilmeyeceği seçilir; kapalıyken yer tutucu gösterilir.
shell-config-media-remote-art-label = Ağdan kapak indir

# Nerede: Ayarlar uygulamasında `audio.max_volume` anahtarının etiketi. Anlam: ses seviyesi üst sınırı (yüzde; 100'ün üstü yükseltme) ayarlanır.
shell-config-audio-max-volume-label = En yüksek ses

# Nerede: Ayarlar uygulamasında `audio.backend` anahtarının etiketi. Anlam: ses seviyesi değişiminin hangi yolla yapılacağı (wpctl ya da süreç içi) seçilir; ayarın deneysel olduğu belirtilir.
shell-config-audio-backend-label = Ses yazım yolu (deneysel)

# Nerede: Ayarlar uygulamasında `osd.timeout_ms` anahtarının etiketi. Anlam: ses/parlaklık göstergesinin son değişimden sonra ekranda kalma süresi (milisaniye) ayarlanır.
shell-config-osd-timeout-label = Gösterge süresi

# Nerede: Ayarlar uygulamasında `wallpaper.path` anahtarının etiketi. Anlam: duvar kağıdı görselinin yolu girilir; boşsa shell duvar kağıdı çizmez.
shell-config-wallpaper-path-label = Duvar kağıdı

# Nerede: Ayarlar uygulamasında `wallpaper.path` alanının altındaki açıklama. Anlam: alana görsel yolu ya da yerleşik Nuxut duvar kağıdı (nuxut:<kimlik>) yazılabileceğini, boş bırakılırsa shell'in duvar kağıdı çizmediğini anlatır; nuxut: öneki ve kimlikler çevrilmez.
shell-config-wallpaper-path-describe = Görsel yolu ya da yerleşik duvar kağıdı (nuxut:pair-flight gibi; koyu/açık tema kipini izler). Boş bırakılırsa duvar kağıdı çizilmez.

# Nerede: Ayarlar uygulamasında `wallpaper.fit` anahtarının etiketi. Anlam: duvar kağıdının ekrana yerleşim biçimi seçilir (doldur/sığdır/oranı koru).
shell-config-wallpaper-fit-label = Yerleşim

# Nerede: Ayarlar uygulamasında `wallpaper.backdrop` anahtarının etiketi. Anlam: duvar kağıdı ekranı tam kaplamadığında kenarlarda kalan zeminin türü seçilir.
shell-config-wallpaper-backdrop-label = Kenar boşluğu zemini

# Nerede: Ayarlar uygulamasında `wallpaper.backdrop_blur` anahtarının etiketi. Anlam: bulanık zemin kopyasının bulanıklık yarıçapı (piksel) ayarlanır.
shell-config-wallpaper-backdrop-blur-label = Zemin bulanıklığı

# Nerede: Ayarlar uygulamasında `wallpaper.backdrop_color` anahtarının etiketi. Anlam: düz zeminin ve geçiş yedeğinin hangi tema renginden boyanacağı seçilir.
shell-config-wallpaper-backdrop-color-label = Zemin rengi

# Nerede: Ayarlar uygulamasında `bar.edge` anahtarının "top" seçeneğinin görünen adı. Anlam: barın ekranın üst kenarına yerleşmesi.
shell-schema-bar-edge-top = Üst

# Nerede: Ayarlar uygulamasında `bar.edge` anahtarının "right" seçeneğinin görünen adı. Anlam: barın ekranın sağ kenarına yerleşmesi.
shell-schema-bar-edge-right = Sağ

# Nerede: Ayarlar uygulamasında `bar.edge` anahtarının "bottom" seçeneğinin görünen adı. Anlam: barın ekranın alt kenarına yerleşmesi.
shell-schema-bar-edge-bottom = Alt

# Nerede: Ayarlar uygulamasında `bar.edge` anahtarının "left" seçeneğinin görünen adı. Anlam: barın ekranın sol kenarına yerleşmesi.
shell-schema-bar-edge-left = Sol

# Nerede: Ayarlar uygulamasında `wallpaper.fit` anahtarının "cover" seçeneğinin görünen adı. Anlam: görsel ekranı doldurur, taşan kısmı kırpılır.
shell-schema-wallpaper-fit-cover = Doldur ve kırp

# Nerede: Ayarlar uygulamasında `wallpaper.fit` anahtarının "contain" seçeneğinin görünen adı. Anlam: görselin tamamı görünür, kenarlarda boşluk kalabilir.
shell-schema-wallpaper-fit-contain = Tamamı

# Nerede: Ayarlar uygulamasında `wallpaper.fit` anahtarının "fill" seçeneğinin görünen adı. Anlam: görsel oranı bozarak ekranı doldurur.
shell-schema-wallpaper-fit-fill = Uzat

# Nerede: Ayarlar uygulamasında `wallpaper.backdrop` anahtarının "blur" seçeneğinin görünen adı. Anlam: zemin, aynı görselin ekranı kaplayan bulanık kopyasıdır.
shell-schema-wallpaper-backdrop-blur = Bulanık kopya

# Nerede: Ayarlar uygulamasında `wallpaper.backdrop` anahtarının "color" seçeneğinin görünen adı. Anlam: zemin, düz tema renginden boyanır.
shell-schema-wallpaper-backdrop-color = Düz renk

# Nerede: Ayarlar uygulamasında `wallpaper.backdrop_color` anahtarının "surface" seçeneğinin görünen adı. Anlam: temanın yüzey rengi.
shell-schema-wallpaper-backdrop-color-surface = Yüzey

# Nerede: Ayarlar uygulamasında `wallpaper.backdrop_color` anahtarının "surface_container" seçeneğinin görünen adı. Anlam: temanın yüzey kapsayıcısı rengi.
shell-schema-wallpaper-backdrop-color-surface-container = Yüzey kapsayıcısı

# Nerede: Ayarlar uygulamasında `wallpaper.backdrop_color` anahtarının "surface_bright" seçeneğinin görünen adı. Anlam: temanın parlak yüzey rengi.
shell-schema-wallpaper-backdrop-color-surface-bright = Parlak yüzey

# Nerede: Ayarlar uygulamasında `audio.backend` anahtarının "wpctl" seçeneğinin görünen adı. Anlam: ses yazımı wpctl alt süreciyle yapılır.
shell-schema-audio-backend-wpctl = wpctl komutu

# Nerede: Ayarlar uygulamasında `audio.backend` anahtarının "native" seçeneğinin görünen adı. Anlam: ses yazımı süreç içinde (WirePlumber mixer-api) yapılır.
shell-schema-audio-backend-native = Süreç içi

# Nerede: Birleşik görünüm anlık görüntüsünde `look.mode` ayarının etiketi (src/services/look/plan.rs). Anlam: uygulama renkleri ile koyu/açık görünümün birlikte yazıldığı ayar.
shell-look-mode-label = Uygulama renkleri ve koyu/açık görünüm

# Nerede: Birleşik görünüm anlık görüntüsünde `look.icons` ayarının etiketi (src/services/look/plan.rs). Anlam: GTK ve Qt uygulamalarının ikon temalarının yazıldığı ayar.
shell-look-icons-label = İkon teması

# Nerede: Birleşik görünüm anlık görüntüsünde `look.cursor` ayarının etiketi (src/services/look/plan.rs). Anlam: imleç teması ve boyunun yazıldığı ayar.
shell-look-cursor-label = İmleç

# Nerede: Birleşik görünüm anlık görüntüsünde `look.fonts` ayarının etiketi (src/services/look/plan.rs). Anlam: arayüz yazı tipleri ile metin boyutu ölçeğinin yazıldığı ayar.
shell-look-fonts-label = Yazı tipleri ve metin boyutu

# Nerede: Birleşik görünüm anlık görüntüsünde `look.motion` ayarının etiketi (src/services/look/plan.rs). Anlam: GTK uygulamalarındaki animasyonların açılıp kapatıldığı ayar.
shell-look-motion-label = Uygulama animasyonları

# Nerede: Birleşik görünüm anlık görüntüsünde `look.qt_backend` ayarının etiketi (src/services/look/plan.rs). Anlam: Qt uygulamalarının temalara hangi yolla uyacağının (KDE ya da qt6ct) seçildiği ayar.
shell-look-qt-backend-label = Qt uygulamalarının görünüm yolu

# Nerede: Birleşik görünüm anlık görüntüsünde `look.unified` ayarının etiketi (src/services/look/plan.rs). Anlam: birleşik görünümün oturum ortamı dosyasının yazıldığı ayar.
shell-look-unified-label = Birleşik görünüm ortamı

## Boşta kalma

# Nerede: Ayarlar uygulamasında `idle.enabled` ana anahtarının etiketi. Anlam: aşağıdaki tüm boşta eylemlerini birden açar/kapatır.
shell-config-idle-enabled-label = Boşta kalma eylemleri

# Nerede: Ayarlar uygulamasında `idle.enabled` ana anahtarının açıklaması. Anlam: kapalıyken boşta eylemleri ve uykudan önce kilit çalışmaz; başka bir boşta kalma aracı kullanılabilir.
shell-config-idle-enabled-description = Kapalıyken boşta eylemlerinin hiçbiri yapılmaz, uykudan önce kilit de çalışmaz; boşta eylemlerini başka bir araç yönetebilir.

# Nerede: Ayarlar uygulamasında `idle.dim` anahtarının etiketi. Anlam: boşta kalınca ekran yarı saydam karartılır.
shell-config-idle-dim-label = Ekranı karart

# Nerede: Ayarlar uygulamasında `idle.dim_after` anahtarının etiketi. Anlam: kaç saniye sonra karartılacağı.
shell-config-idle-dim-after-label = Karartma süresi

# Nerede: Ayarlar uygulamasında `idle.screen_off` anahtarının etiketi. Anlam: boşta kalınca monitörler kapatılır (DPMS).
shell-config-idle-screen-off-label = Ekranı kapat

# Nerede: Ayarlar uygulamasında `idle.screen_off_after` anahtarının etiketi. Anlam: kaç saniye sonra ekranın kapatılacağı.
shell-config-idle-screen-off-after-label = Ekranı kapatma süresi

# Nerede: Ayarlar uygulamasında `idle.lock` anahtarının etiketi. Anlam: boşta kalınca oturum kilitlenir (uykudan önce kilit ayrı ayar).
shell-config-idle-lock-label = Boşta kilitle

# Nerede: Ayarlar uygulamasında `idle.lock` anahtarının açıklaması. Anlam: kilit ekranını Nuxut açar (lock.listen_logind).
shell-config-idle-lock-description = Belirlenen süre boşta kalınca oturumu kilitler.

# Nerede: Ayarlar uygulamasında `idle.lock_after` anahtarının etiketi. Anlam: kaç saniye sonra oturumun kilitleneceği.
shell-config-idle-lock-after-label = Kilitleme süresi

# Nerede: Ayarlar uygulamasında `lock.listen_logind` anahtarının etiketi. Anlam: oturum yöneticisi (logind) kilit isteyince Nuxut'un kendi kilit ekranını açması.
shell-config-lock-listen-logind-label = Nuxut kilit ekranını kullan

# Nerede: Ayarlar uygulamasında `lock.listen_logind` anahtarının açıklaması. Anlam: kapatılırsa kilit ekranını kullanıcının kendi aracı açar.
shell-config-lock-listen-logind-description = Kilitle düğmesi, kısayolu ve otomatik kilit Nuxut kilit ekranını açar. Başka bir kilit ekranı kullanılacaksa kapatılır.

# Nerede: Ayarlar uygulamasında `idle.lock_after` anahtarının açıklaması. Anlam: 0 değeri verilirse boşta kilitlenmez (uykudan önce kilit ayrı ayardır).
shell-config-idle-lock-after-description = 0 = boşta kilitleme.

# Nerede: Ayarlar uygulamasında `idle.lock_before_sleep` anahtarının etiketi. Anlam: bilgisayar uykuya (askıya alma) geçmeden önce oturum kilitlenir.
shell-config-idle-lock-before-sleep-label = Uykudan önce kilitle

# Nerede: Ayarlar uygulamasında `idle.lock_before_sleep` anahtarının açıklaması. Anlam: kilit ekranı açılana dek uyku birkaç saniye bekletilir; uyanınca masaüstü kilitli gelir.
shell-config-idle-lock-before-sleep-description = Bilgisayar uyumadan önce kilit ekranı açılır; uyandığında masaüstü kilitli gelir.

# Nerede: Ayarlar uygulamasında `idle.suspend` anahtarının etiketi. Anlam: boşta kalınca makine askıya alınır (uyku).
shell-config-idle-suspend-label = Uyu

# Nerede: Ayarlar uygulamasında `idle.suspend` anahtarının açıklaması. Anlam: prizdeyken uyumaz, ayrı bir ayarla açılır.
shell-config-idle-suspend-description = Varsayılan olarak yalnızca pildeyken.

# Nerede: Ayarlar uygulamasında `idle.suspend_after` anahtarının etiketi. Anlam: kaç saniye sonra makinenin uykuya geçeceği.
shell-config-idle-suspend-after-label = Uyku süresi

# Nerede: Ayarlar uygulamasında `idle.suspend_on_ac` anahtarının etiketi. Anlam: şarj aletine takılıyken de uyusun.
shell-config-idle-suspend-on-ac-label = Prizdeyken de uyu

# Nerede: Ayarlar uygulamasında `idle.media_inhibits` anahtarının etiketi. Anlam: müzik/video çalarken boşta eylemleri yapılmaz.
shell-config-idle-media-inhibits-label = Medya oynarken bekle

# Nerede: Ayarlar uygulamasında `idle.media_inhibits` anahtarının açıklaması. Anlam: müzik ya da video çalarken ekran kararmaz, kilitlenmez.
shell-config-idle-media-inhibits-description = Müzik ya da video çalarken ekran kararmaz, kilitlenmez.

# Nerede: shell.toml doğrulama uyarısı (log ve bildirim). Anlam: anahtara yazılan değer geçerli aralıkta değil, kabul edilebilir değer kullanılıyor.
# Değişkenler: $key (ayar anahtarı, örnek: idle.dim_after), $value (yazılan değer, örnek: 5), $fixed (kullanılan değer, örnek: 30), $range (geçerli aralık, örnek: 30..=14400).
shell-config-idle-out-of-range = { $key } = { $value } aralık dışı, { $fixed } kullanılıyor ({ $range })

# Nerede: logind uyku gecikme inhibitörünün gerekçe metni; `systemd-inhibit --list` çıktısında ve bazı masaüstlerinin kapanış/uyku pencerelerinde görünür. Anlam: Nuxut, uykudan önce kilit ekranının kurulmasını beklemek için uykuyu birkaç saniye geciktirir.
shell-idle-sleep-inhibit-why = Uykudan önce oturum kilitlenir

## Sürüm göçü bildirimi

# Nerede: shell.toml eski sürümden yeniye göç ettikten sonra bildirim merkezine düşen sessiz bildirimin başlığı. Anlam: ayarların yeni config sürümüne otomatik taşındığı bildirilir.
shell-config-migrated-title = Ayarların yeni sürüme taşındı

# Nerede: Göç bildiriminin gövdesinde ve Ayarlar'ın "Bu sürümde yeni" listesinde değişen ayarın adı; `bar.left`/`bar.right` → `bar.start`/`bar.end` değişikliği. Anlam: barın uç widget listelerinin (bölümlerinin) yeniden adlandırıldığı.
shell-config-migrate-note-bar-section = Bar bölümü

# Nerede: Göç bildiriminin gövdesinde ve Ayarlar'ın "Bu sürümde yeni" listesinde değişen ayarın adı; `[workspaces]`/`[clock]` tablolarının `[bar.widget.*]` altına taşınması. Anlam: widget'a özel ayarların yeni tabloya taşındığı.
shell-config-migrate-note-widget-settings = Widget ayarları

# Nerede: Göç bildiriminin gövdesinde ve Ayarlar'ın "Bu sürümde yeni" listesinde değişen ayarın adı; etkisiz `frame.inner_radius` anahtarının silinmesi. Anlam: çerçevenin iç köşe yuvarlaması ayarının kalktığı.
shell-config-migrate-note-frame-inner-corner = Çerçeve iç köşesi

# Nerede: Göç bildiriminin gövdesinde ve Ayarlar'ın "Bu sürümde yeni" listesinde değişen ayarın adı; workspaces `show_empty` anahtarının kaldırılması. Anlam: boş workspace yuvalarını gösterme ayarının kalktığı (yeni anlamda her sayfa dolu çizilir).
shell-config-migrate-note-show-empty = Boşları göster

# Nerede: Ayarlar'ın geçmiş/anlık görüntü listesinde göç snapshot'ının etiketi; shell.toml sürüm göçü geri alınabilir değişikliklarla yazıldığında snapshot alınırken (src/config_migrate.rs). Anlam: shell.toml'un eski config sürümünden yeniye göçtüğü.
# Değişkenler: $from (eski sürüm, örnek: 1), $to (yeni sürüm, örnek: 2).
shell-config-migrate-snapshot-label = shell.toml sürüm göçü { $from }→{ $to }
# Nerede: shell.toml sürüm göçü bildirimindeki eylem düğmesi; basınca göçün davranış değiştiren düzenlemeleri geri alınır (src/config_migrate.rs). Anlam: göçün yaptığı değişiklikler geri alınıp ayarlar göç öncesi hâline döndürülür.
shell-config-migrated-undo = Geri al

## Ayar onarımı (kendini onarma)

# Nerede: Ayar dosyasında (shell.toml, framework.toml) okunamayan satırın üstüne yazılan yorum (src/config_heal.rs). Anlam: satır bozuktu, okunamadığı için yoruma alındı; ardından tarih gelir.
shell-config-heal-mark-broken = nuxut: okunamadı, yoruma alındı
# Nerede: Ayar dosyasında aynı anahtar iki kez yazıldığında öncekinin üstüne yazılan yorum. Anlam: bu satır tekrar ediyordu; aşağıdaki (son yazılan) değer geçerli.
shell-config-heal-mark-duplicate = nuxut: tekrar eden anahtar, alttaki geçerli
# Nerede: Ayar dosyasında geçersiz değerin (yanlış tür, bilinmeyen seçenek) üstüne yazılan yorum. Anlam: değer kullanılamadı, yerine varsayılan geçerli; eski değer altta yorum olarak durur.
shell-config-heal-mark-invalid = nuxut: geçersiz değer, varsayılan kullanılıyor
# Nerede: Ayar dosyasında izin verilen aralığın dışındaki sayının yerine sınır değeri yazıldığında eski satırın yorum hâli. Anlam: değer sınır dışıydı; zaten kullanılan sınır değeri yazıldı.
shell-config-heal-mark-clamped = nuxut: sınır dışıydı, sınır değeri yazıldı
# Nerede: Açılışta ayar dosyası onarılıp diske yazıldığında bildirim merkezine düşen sessiz bildirimin başlığı. Anlam: ayarlardaki şu kadar satır düzeltildi.
# Değişkenler: $count (düzeltilen satır sayısı, örnek: 3).
shell-config-healed-title = Ayarlarında { $count } satır düzeltildi
# Nerede: Onarım yalnızca eski/kullanılmayan satırları sildiğinde bildirimin başlığı. Anlam: ayarlardaki şu kadar eski satır temizlendi.
# Değişkenler: $count (silinen satır sayısı, örnek: 3).
shell-config-healed-cleaned-title = Ayarlarında { $count } eski satır temizlendi
# Nerede: Ayar dosyası elle değiştirildikten sonra onarılabilecek satırlar bulunduğunda (henüz yazılmadı) bildirimin başlığı. Anlam: ayarlarda düzeltilebilecek şu kadar satır var.
# Değişkenler: $count (satır sayısı, örnek: 2).
shell-config-heal-pending-title = Ayarlarında düzeltilebilecek { $count } satır var
# Nerede: Onarım bildiriminin gövdesindeki parça. Anlam: şu kadar eski ya da tanınmayan satır silindi.
# Değişkenler: $count (sayı, örnek: 2).
shell-config-heal-part-cleaned = { $count } eski satır temizlendi
# Nerede: Onarım bildiriminin gövdesindeki parça. Anlam: şu kadar geçersiz değer varsayılan değere döndü.
# Değişkenler: $count (sayı, örnek: 1).
shell-config-heal-part-reset = { $count } geçersiz değer varsayılana döndü
# Nerede: Onarım bildiriminin gövdesindeki parça. Anlam: şu kadar bozuk satır yoruma alındı (okunabilen kısım kurtarıldı).
# Değişkenler: $count (sayı, örnek: 1).
shell-config-heal-part-repaired = { $count } bozuk satır yoruma alındı
# Nerede: Onarım bildirimindeki eylem düğmesi. Anlam: onarımı geri al; dosya onarım öncesi hâline döner.
shell-config-heal-undo = Geri al
# Nerede: Bekleyen onarım bildirimindeki eylem düğmesi. Anlam: önerilen düzeltmeleri şimdi uygula (önce yedek alınır).
shell-config-heal-fix = Onar
# Nerede: Ayar dosyasında yazım hatasına benzeyen anahtar bulunduğunda bildirimin başlığı. Anlam: şu kadar ayar adı tanınmadı.
# Değişkenler: $count (sayı, örnek: 1).
shell-config-heal-typo-title = { $count } ayar adı tanınmadı
# Nerede: Yazım hatası bildiriminin gövdesinde bir öneri. Anlam: yazılan ad ve önerilen doğru ad.
# Değişkenler: $key (yazılan anahtar, örnek: bar.siez), $candidate (önerilen ad, örnek: size).
shell-config-heal-typo-item = { $key } → { $candidate }
# Nerede: Yazım hatası bildirimindeki eylem düğmesi. Anlam: önerilen adları uygula.
shell-config-heal-typo-fix = Düzelt
# Nerede: Yazım hatası bildirimindeki eylem düğmesi. Anlam: önerileri yok say, bu anahtarlara bir daha dokunma.
shell-config-heal-typo-ignore = Yok say

## Sistem izleyici

# Nerede: Sistem izleyici popout'unun sol sütunundaki ilk bölüm başlığı (src/sysmon_view.rs). Anlam: işlemci kullanım bölümü.
shell-sysmon-title-cpu = İşlemci

# Nerede: Sistem izleyici popout'unun sol sütunundaki bölüm başlığı (src/sysmon_view.rs). Anlam: bellek kullanım bölümü.
shell-sysmon-title-memory = Bellek

# Nerede: Sistem izleyici popout'unun sağ sütunundaki bölüm başlığı (src/sysmon_view.rs). Anlam: sıcaklık bölümü.
shell-sysmon-title-temperature = Sıcaklık

# Nerede: Sistem izleyici popout'unun sağ sütunundaki bölüm başlığı (src/sysmon_view.rs). Anlam: ağ hızı bölümü.
shell-sysmon-title-network = Ağ

# Nerede: Sistem izleyici popout'unda ağ bölümündeki satır etiketi (src/sysmon_view.rs). Anlam: indirme (aşağı yönlü) hızı.
shell-sysmon-title-download = İndirme

# Nerede: Sistem izleyici popout'unda ağ bölümündeki satır etiketi (src/sysmon_view.rs). Anlam: yükleme (yukarı yönlü) hızı.
shell-sysmon-title-upload = Yükleme

# Nerede: Sistem izleyici popout'unun sağ sütunundaki bölüm başlığı (src/sysmon_view.rs). Anlam: disk bölümü.
shell-sysmon-title-disk = Disk

# Nerede: Sistem izleyici popout'unun en altındaki tam genişlikte bölüm başlığı (src/sysmon_view.rs). Anlam: işlemciyi en çok kullanan süreçlerin listesi.
shell-sysmon-title-top-processes = En çok işlemci kullananlar

# Nerede: Sistem izleyici popout'unda işlemci bölümünün alt notu; çekirdek çubuğu gösterilemeyen çekirdekler varken (src/sysmon_view.rs). Anlam: çubukta gösterilmeyen çekirdek sayısı.
# Değişkenler: $count (gösterilmeyen çekirdek sayısı, örnek: 8).
shell-sysmon-note-extra-cores = +{ $count } çekirdek

# Nerede: Sistem izleyici popout'unda işlemci bölümünün alt notu; yük ortalaması ölçümü varken (src/sysmon_view.rs). Anlam: 1, 5 ve 15 dakikalık yük ortalamaları.
# Değişkenler: $load1 (1 dakikalık yük ortalaması, önceden biçimli, örnek: "0,50"), $load5 (5 dakikalık yük ortalaması, önceden biçimli, örnek: "0,25"), $load15 (15 dakikalık yük ortalaması, önceden biçimli, örnek: "1,00").
shell-sysmon-note-load = Yük { $load1 } { $load5 } { $load15 }

# Nerede: Sistem izleyici popout'unda bellek bölümünün alt notu; takas alanı varken (src/sysmon_view.rs). Anlam: takas alanının kullanımı.
# Değişkenler: $usage (takas kullanımı, önceden biçimli, örnek: "0,0 / 8,0 GiB").
shell-sysmon-note-swap = Takas { $usage }

# Nerede: Sistem izleyici popout'unda GPU bölümünün sağ değeri; GPU askıdayken (src/sysmon_view.rs). Anlam: GPU uyku durumunda, kullanım ölçülmüyor.
shell-sysmon-gpu-suspended = uykuda

# Nerede: Sistem izleyici popout'unda disk bölümünün alt notu; disk G/Ç ölçümü varken (src/sysmon_view.rs). Anlam: saniyelik okuma ve yazma hızları.
# Değişkenler: $read (okuma hızı, önceden biçimli, örnek: "1,2 MiB/s"), $write (yazma hızı, önceden biçimli, örnek: "512 KiB/s").
shell-sysmon-note-disk-io = Okuma { $read } · Yazma { $write }

# Nerede: Bar'daki sistem izleyici göstergesinin ipucunun ilk satırı; işlemci ölçümü varken (src/services/sysmon/format.rs). Anlam: işlemci kullanım yüzdesi; sıcaklık varsa satıra " · " ile eklenir.
# Değişkenler: $value (işlemci kullanım yüzdesi, önceden biçimli, örnek: "%12").
shell-sysmon-tooltip-cpu = İşlemci { $value }

# Nerede: Bar'daki sistem izleyici göstergesinin ipucunun ilk satırı; işlemci ölçümü yokken (src/services/sysmon/format.rs). Anlam: işlemci kullanımı bilinmiyor.
shell-sysmon-tooltip-cpu-unavailable = İşlemci —

# Nerede: Bar'daki sistem izleyici göstergesinin ipucunun bellek satırı (src/services/sysmon/format.rs). Anlam: bellek kullanımı ve kullanım yüzdesi.
# Değişkenler: $usage (bellek kullanımı, önceden biçimli, örnek: "4,0 / 16,0 GiB"), $percent_text (kullanım yüzdesi, önceden biçimli, örnek: "%25").
shell-sysmon-tooltip-memory = Bellek { $usage } ({ $percent_text })

# Nerede: Bar'daki sistem izleyici göstergesinin ipucunun son satırı, ağ hızlarıyla " · " ile birleşik; kök disk ölçümü varken (src/services/sysmon/format.rs). Anlam: kök disk doluluk yüzdesi.
# Değişkenler: $percent_text (kök disk doluluk yüzdesi, önceden biçimli, örnek: "%50").
shell-sysmon-tooltip-disk = Disk { $percent_text }

# Nerede: Sistem izleyici popout'unun ortasındaki satır; ilk ölçüm alınana dek (src/sysmon_view.rs). Anlam: gösterilecek ölçüm henüz yok.
shell-sysmon-waiting = Ölçüm bekleniyor…

# Nerede: Ayarlar uygulamasında sysmon.interval anahtarının satır etiketi; değere sn birimi eklenir (src/services/sysmon/mod.rs). Anlam: sistem izleyicinin ölçüm aralığı.
shell-sysmon-setting-interval-label = Sistem izleyici yenileme aralığı

# Nerede: Ayarlar uygulamasında sysmon.processes anahtarının satır etiketi (src/services/sysmon/mod.rs). Anlam: paneldeki süreç listesinde gösterilen en çok süreç sayısı.
shell-sysmon-setting-processes-label = Panelde gösterilen süreç

## Bar durum göstergeleri

# Nerede: Bar'daki durum göstergelerinde (ses, pil) ikonun yanındaki yüzde değeri. Anlam: seviye yüzdesi; yer en geniş değere ("%100") göre ayrılır.
# Değişkenler: $percent (0–1 arası oran, örnek: 0,64; gösterim katalogda yüzdeye çevrilir).
# [max: 4]
shell-bar-status-percent = { NUMBER($percent, style: "percent", maximumFractionDigits: 0) }

# Nerede: Bar'daki ağ göstergesinin ipucu, ilk satır. Anlam: bağlı Wi-Fi ağının adı ve sinyal gücü.
# Değişkenler: $ssid (ağ adı, örnek: Ev), $strength (sinyal gücü, 0–1 arası oran, örnek: 0,8; gösterim katalogda yüzdeye çevrilir).
shell-bar-status-wifi = Wi-Fi: { $ssid } · { NUMBER($strength, style: "percent", maximumFractionDigits: 0) }

# Nerede: Bar'daki ağ göstergesinin ipucu. Anlam: Wi-Fi bağlı ama erişim noktası bilgisi henüz yok.
shell-bar-status-wifi-connected = Wi-Fi bağlı

# Nerede: Bar'daki ağ göstergesinin ipucu. Anlam: kablolu (Ethernet) bağlantı etkin; bağlantının adıyla.
# Değişkenler: $name (bağlantı adı, örnek: Kablolu bağlantı 1).
shell-bar-status-ethernet-named = Kablolu ağ: { $name }

# Nerede: Bar'daki ağ göstergesinin ipucu. Anlam: kablolu (Ethernet) bağlantı etkin, adı bilinmiyor.
shell-bar-status-ethernet = Kablolu ağ

# Nerede: Bar'daki ağ göstergesinin ipucu. Anlam: Wi-Fi ya da kablo dışında bir bağlantı etkin (ör. mobil); bağlantının adıyla.
# Değişkenler: $name (bağlantı adı, örnek: Mobil).
shell-bar-status-connected-named = Bağlı: { $name }

# Nerede: Bar'daki ağ göstergesinin ipucu. Anlam: bir ağa bağlı, türü ve adı bilinmiyor.
shell-bar-status-connected = Bağlı

# Nerede: Bar'daki ağ göstergesinin ipucu. Anlam: ağ bağlantısı kuruluyor.
shell-bar-status-connecting = Bağlanıyor…

# Nerede: Bar'daki ağ göstergesinin ipucu. Anlam: Wi-Fi radyosu kapalı ve hiçbir ağa bağlı değil.
shell-bar-status-wifi-off = Wi-Fi kapalı · bağlantı yok

# Nerede: Bar'daki ağ göstergesinin ipucu. Anlam: hiçbir ağa bağlı değil.
shell-bar-status-offline = Bağlantı yok

# Nerede: Bar'daki ağ göstergesinin ipucu, ek satır. Anlam: yerel ağa bağlı ama internete çıkılamıyor.
shell-bar-status-no-internet = İnternet erişimi yok

# Nerede: Bar'daki ağ göstergesinin ipucu, ek satır. Anlam: ağ, internet için tarayıcıda giriş sayfası (captive portal) istiyor.
shell-bar-status-portal = Ağa giriş gerekiyor

# Nerede: Bar'daki ağ göstergesinin ipucu, ek satır (her etkin VPN için bir satır). Anlam: etkin VPN bağlantısı.
# Değişkenler: $name (VPN bağlantısının adı, örnek: İş).
shell-bar-status-vpn = VPN: { $name }

# Nerede: Bar'daki Bluetooth göstergesinin ipucu. Anlam: Bluetooth açık, bağlı cihaz yok.
shell-bar-status-bluetooth-on = Bluetooth açık

# Nerede: Bar'daki Bluetooth göstergesinin ipucu, ilk satır (altında cihaz adları listelenir). Anlam: bağlı cihaz sayısı.
# Değişkenler: $count (bağlı cihaz sayısı, örnek: 2).
shell-bar-status-bluetooth-connected = Bluetooth: { $count } cihaz bağlı

# Nerede: Bar'daki Bluetooth göstergesinin ipucu, cihaz satırı. Anlam: bağlı cihazın adı ve kendi pil seviyesi.
# Değişkenler: $name (cihaz adı, örnek: Kulaklık), $percent (cihazın pil düzeyi, 0–1 arası oran, örnek: 0,8; gösterim katalogda yüzdeye çevrilir).
shell-bar-status-bluetooth-device-battery = { $name } · pil { NUMBER($percent, style: "percent", maximumFractionDigits: 0) }

# Nerede: Bar'daki Bluetooth göstergesinin ipucu, son satır. Anlam: listede gösterilmeyen bağlı cihaz sayısı.
# Değişkenler: $count (gösterilmeyen cihaz sayısı, örnek: 2).
shell-bar-status-bluetooth-more = ve { $count } cihaz daha

# Nerede: Bar'daki ses göstergesinin ipucu, ilk satır (altında çıkış cihazının adı). Anlam: ses seviyesi.
# Değişkenler: $percent (ses düzeyi, 0–1 arası oran, örnek: 0,45; gösterim katalogda yüzdeye çevrilir).
shell-bar-status-volume = Ses { NUMBER($percent, style: "percent", maximumFractionDigits: 0) }

# Nerede: Bar'daki ses göstergesinin ipucu, ilk satır. Anlam: ses kapatılmış (sessiz); parantez içinde kapatılmadan önceki seviye.
# Değişkenler: $percent (ses düzeyi, 0–1 arası oran, örnek: 0,45; gösterim katalogda yüzdeye çevrilir).
shell-bar-status-volume-muted = Ses kapalı ({ NUMBER($percent, style: "percent", maximumFractionDigits: 0) })

# Nerede: Bar'daki pil göstergesinin ipucu, başı (ardından " · " ile durum gelebilir). Anlam: pil seviyesi.
# Değişkenler: $percent (pil düzeyi, 0–1 arası oran, örnek: 0,64; gösterim katalogda yüzdeye çevrilir).
shell-bar-status-battery = Pil { NUMBER($percent, style: "percent", maximumFractionDigits: 0) }

# Nerede: Bar'daki pil göstergesinin ipucunda pil seviyesinden sonra. Anlam: pil tam dolu.
shell-bar-status-battery-full = Dolu

# Nerede: Bar'daki pil göstergesinin ipucunda pil seviyesinden sonra. Anlam: şarj oluyor, dolma süresi bilinmiyor.
shell-bar-status-battery-charging = Şarj oluyor

# Nerede: Bar'daki pil göstergesinin ipucunda pil seviyesinden sonra. Anlam: şarj oluyor ve tahmini dolma süresi.
# Değişkenler: $time (biçimlenmiş süre, örnek: 1 sa 30 dk).
shell-bar-status-battery-charging-time = Şarj oluyor · dolmasına { $time }

# Nerede: Bar'daki pil göstergesinin ipucunda pil seviyesinden sonra. Anlam: pille çalışırken tahmini kalan süre.
# Değişkenler: $time (biçimlenmiş süre, örnek: 1 sa 30 dk).
shell-bar-status-battery-remaining = { $time } kaldı

## Ekran görüntüsü

# Nerede: Ayarlar uygulamasında `screenshot.dir` metin alanının etiketi. Anlam: ekran görüntülerinin kaydedildiği klasör.
shell-config-screenshot-dir-label = Ekran görüntüsü klasörü

# Nerede: Ayarlar uygulamasında `screenshot.dir` alanının altındaki açıklama. Anlam: boş bırakılırsa Resimler klasörünün içindeki varsayılan alt klasör kullanılır; yol mutlak ya da ~/ ile başlamalıdır.
shell-config-screenshot-dir-describe = Boşsa Resimler içinde "Ekran Görüntüleri" kullanılır. Mutlak ya da ~/ ile başlayan bir yol.

# Nerede: Ayarlar uygulamasında `screenshot.save` anahtarının etiketi. Anlam: ekran görüntüsü dosyaya kaydedilsin mi.
shell-config-screenshot-save-label = Ekran görüntüsünü kaydet

# Nerede: Ayarlar uygulamasında `screenshot.copy` anahtarının etiketi. Anlam: ekran görüntüsü panoya kopyalansın mı.
shell-config-screenshot-copy-label = Ekran görüntüsünü panoya kopyala

# Nerede: Ayarlar uygulamasında `screenshot.notify` anahtarının etiketi. Anlam: ekran görüntüsü alınınca bildirim gösterilsin mi.
shell-config-screenshot-notify-label = Ekran görüntüsü bildirimi

# Nerede: shell.toml doğrulama uyarısı (log ve bildirim). Anlam: screenshot.dir geçersiz (göreli yol ya da ".." içeriyor), varsayılan klasör kullanılıyor.
shell-config-screenshot-dir-invalid = screenshot.dir mutlak ya da ~/ ile başlayan bir yol olmalı; varsayılan klasör kullanılıyor

# Nerede: shell.toml doğrulama uyarısı (log ve bildirim). Anlam: hem kaydetme hem kopyalama kapatılmış; ekran görüntüsü bir işe yaramayacağı için kaydetme açıldı.
shell-config-screenshot-nothing-enabled = screenshot.save ve screenshot.copy ikisi de kapalı; kaydetme açıldı

# Nerede: Ayarlar uygulamasında `clipboard.enabled` anahtarının etiketi. Anlam: kopyalanan metin ve görsellerin geçmişi tutulsun mu.
shell-config-clipboard-enabled-label = Pano geçmişi

# Nerede: Ayarlar uygulamasında `clipboard.max_items` alanının etiketi. Anlam: geçmişte en çok kaç öğe saklanır; fazlası olunca en eskiler silinir.
shell-config-clipboard-max-items-label = Saklanan öğe sayısı

# Nerede: Ayarlar uygulamasında `clipboard.images` anahtarının etiketi. Anlam: kopyalanan görseller (PNG, JPEG) de geçmişe alınsın mı.
shell-config-clipboard-images-label = Görselleri de sakla

# Nerede: Ayarlar uygulamasında `clipboard.persist` anahtarının etiketi. Anlam: pano geçmişi diske yazılsın ve yeniden başlatmadan sonra da kalsın mı.
shell-config-clipboard-persist-label = Geçmişi diske kaydet

# Nerede: Ayarlar uygulamasında `clipboard.persist` anahtarının altındaki açıklama. Anlam: kapalıyken geçmiş yalnızca bellekte; açıkken diskte kalır; SSD'de silinen dosyanın izi fiziksel olarak kalabilir.
shell-config-clipboard-persist-describe = Kapalıyken geçmiş yalnızca bellekte tutulur. Açıkken yeniden başlatmadan sonra da kalır; SSD'lerde silinen dosyanın izleri fiziksel olarak kalabilir.

# Nerede: Ayarlar uygulamasında `clipboard.primary` anahtarının etiketi. Anlam: fareyle seçilen (orta tıkla yapıştırılan) metin de geçmişe alınsın mı.
shell-config-clipboard-primary-label = Seçilen metni de sakla

# Nerede: Ayarlar uygulamasında `clipboard.primary` anahtarının altındaki açıklama. Anlam: birincil seçim (seçip orta tıkla yapıştırma) her seçimde değişir; açıkken o metinler de geçmişe girer.
shell-config-clipboard-primary-describe = Fareyle seçilip orta tıkla yapıştırılan metin de geçmişe girer. Her seçimde değiştiği için geçmiş hızla dolabilir.

# Nerede: Ayarlar uygulamasında `clipboard.x11_bridge` seçiminin başlığı. Anlam: sanal makinenin konuk ajanı (SPICE, VMware, VirtualBox) ile pano paylaşımı için X11 ↔ Wayland pano eşitlemesi.
shell-config-clipboard-x11-bridge-label = Sanal makine pano paylaşımı

# Nerede: Ayarlar uygulamasında `clipboard.x11_bridge` altındaki açıklama. Anlam: konuk ajanları panoyu yalnız X11 ile konuşur; köprü X11 uygulamalarının odakta olmadan panoya erişmesine izin verir, bu yüzden otomatikte yalnız sanal makinede açılır.
shell-config-clipboard-x11-bridge-describe = Sanal makinede ana bilgisayarla kopyala-yapıştır için X11 panosunu Wayland panosuyla eşitler. Otomatik yalnızca bir sanal makine ajanı çalışırken açılır ve panoyu yalnız o ajana açar (odaktaki X11 uygulamalarını Hyprland kendisi eşitler); Her zaman ise odakta olmayan her X11 uygulamasına da açar.

# Nerede: `clipboard.x11_bridge` seçeneği. Anlam: yalnız sanal makine konuk ajanı varken açık.
shell-config-clipboard-x11-bridge-auto = Otomatik (sanal makinede)

# Nerede: `clipboard.x11_bridge` seçeneği. Anlam: her zaman açık.
shell-config-clipboard-x11-bridge-on = Her zaman

# Nerede: `clipboard.x11_bridge` seçeneği. Anlam: kapalı.
shell-config-clipboard-x11-bridge-off = Kapalı

# Nerede: shell.toml doğrulama uyarısı (log ve bildirim). Anlam: clipboard.max_items izin verilen aralığın dışında; en yakın sınır kullanılıyor.
# Değişkenler: $value (kullanılan değer, örnek: 500).
shell-config-clipboard-max-items-clamped = clipboard.max_items 10–500 aralığının dışında; { $value } kullanılıyor

# Nerede: shell.toml doğrulama uyarısı (log ve bildirim). Anlam: clipboard.exclude_apps listesindeki geçersiz adlar (boş, çok uzun ya da kontrol karakterli) ya da sınırı aşan fazlalık atıldı.
shell-config-clipboard-exclude-apps-invalid = clipboard.exclude_apps içindeki geçersiz adlar atıldı

# Nerede: Ayarlar uygulamasında `clipboard.exclude_apps` listesinin etiketi. Anlam: kopyalama anında bu uygulamalardan biri etkin penceredeyse kopyalanan şey geçmişe alınmaz (ör. parola yöneticileri).
shell-config-clipboard-exclude-apps-label = Geçmişe alınmayan uygulamalar

# Nerede: Ayarlar uygulamasında `clipboard.exclude_apps` listesinin altındaki açıklama. Anlam: liste pencere sınıfı adlarıdır; etkin pencereye bakılır, bu yüzden kesin bir koruma değildir.
shell-config-clipboard-exclude-apps-describe = Pencere sınıfı adları. Kopyalama anındaki etkin pencereye bakılır; kesin bir koruma değildir.

# Nerede: Resimler klasörü içinde ekran görüntülerinin kaydedildiği alt klasörün adı (dosya sisteminde görünür). Anlam: ekran görüntüleri klasörü. Eğik çizgi içeremez.
shell-screenshot-dir-name = Ekran Görüntüleri

# Nerede: Kaydedilen ekran görüntüsü dosyasının adı (uzantısız; dosya sisteminde görünür). Anlam: "ekran görüntüsü" ve çekildiği an. Eğik çizgi içeremez.
# Değişkenler: $time (tarih ve saat, örnek: 2026-10-12 14-03-22).
shell-screenshot-file-name = Ekran görüntüsü { $time }

# Nerede: Ekran görüntüsü seçim katmanında seçimin altındaki boyut etiketi. Anlam: kaydedilecek görüntünün piksel genişliği ve yüksekliği.
# Değişkenler: $width (piksel, örnek: 1920), $height (piksel, örnek: 1080).
shell-screenshot-size = { $width } × { $height }

# Nerede: Ekran görüntüsü seçim katmanının altında, henüz seçim yokken görünen ipucu. Anlam: fareyle sürükleyerek bölge seçilir, tıklayınca pencere alınır, Enter onaylar, Esc iptal eder.
# [max: 80]
shell-screenshot-hint = Sürükle: bölge · Tıkla: pencere · Enter: onayla · Esc: iptal

# Nerede: Ekran görüntüsü bildiriminin başlığı. Anlam: ekran görüntüsü dosyaya kaydedildi.
shell-screenshot-saved-title = Ekran görüntüsü kaydedildi

# Nerede: Ekran görüntüsü bildiriminin başlığı (dosyaya kaydedilmeden yalnızca kopyalandığında). Anlam: ekran görüntüsü panoya kopyalandı.
shell-screenshot-copied-title = Ekran görüntüsü panoya kopyalandı

# Nerede: Ekran görüntüsü bildiriminin başlığı (hata). Anlam: ekran görüntüsü alınamadı ya da kaydedilemedi; gövdede neden yazar.
shell-screenshot-failed-title = Ekran görüntüsü alınamadı

# Nerede: Ekran görüntüsü bildiriminin gövdesinde dosya adının altındaki satır. Anlam: görüntü panoya da kopyalandı.
shell-screenshot-copied = Panoya kopyalandı

# Nerede: Ekran görüntüsü bildirimindeki düğme. Anlam: kaydedilen görüntüyü varsayılan uygulamada aç.
# [max: 16]
shell-screenshot-action-open = Aç

# Nerede: Ekran görüntüsü bildirimindeki düğme. Anlam: kaydedilen görüntüyü dosya yöneticisinde, klasöründe seçili göster.
# [max: 20]
shell-screenshot-action-reveal = Klasörde göster

## Girdi aygıtları ([input])

# Nerede: Ayarlar uygulamasında compositor.sync_input anahtarının satır etiketi. Anlam: shell.toml [input] ayarlarının Hyprland'e uygulanması.
shell-hypr-sync-input-label = Klavye, fare ve dokunmatik yüzey ayarlarını uygula

# Nerede: Ayarlar uygulamasında compositor.base_layer anahtarının satır etiketi. Anlam: Nuxut'un Hyprland için hazır pencere yönetimi kısayollarını, pencere kurallarını ve davranış varsayılanlarını (kullanıcının kendi Hyprland ayarı bunları ezebilir) uygulaması.
shell-hypr-base-layer-label = Hazır pencere yönetimi ve kısayollar

# Nerede: Ayarlar uygulamasında compositor.take_over_keys anahtarının satır etiketi. Anlam: kullanıcının kendi Hyprland ayarında ses, parlaklık, medya, ekran görüntüsü gibi tuşlar başka bir şeye bağlıysa Nuxut bu tuşları kendi işlevleri için devralır (kullanıcının dosyası değişmez; kapatınca eski bağlama geri döner).
shell-hypr-take-over-keys-label = Medya, ses ve ekran görüntüsü tuşlarını Nuxut kullansın

# Nerede: Ayarlar uygulamasında input.keyboard.layouts anahtarının etiketi. Anlam: kullanılan klavye düzenleri listesi (ör. Türkçe Q, İngilizce).
shell-config-input-layouts-label = Klavye düzenleri

# Nerede: Ayarlar uygulamasında input.keyboard.options anahtarının etiketi. Anlam: xkb seçenekleri (ör. düzen değiştirme kısayolu, Caps Lock davranışı).
shell-config-input-options-label = Klavye seçenekleri

# Nerede: Ayarlar uygulamasında input.keyboard.repeat_rate anahtarının etiketi; değere "/sn" birimi eklenir. Anlam: basılı tutulan tuşun saniyede kaç kez tekrarlandığı.
shell-config-input-repeat-rate-label = Tuş tekrar hızı

# Nerede: Ayarlar uygulamasında input.keyboard.repeat_delay anahtarının etiketi; değere ms birimi eklenir. Anlam: tuş basılı tutulunca tekrarın başlamasından önceki bekleme.
shell-config-input-repeat-delay-label = Tuş tekrar gecikmesi

# Nerede: Ayarlar uygulamasında input.keyboard.numlock_by_default anahtarının etiketi. Anlam: oturum açılınca Num Lock açık başlar. "Num Lock" tuş adıdır, çevrilmez.
shell-config-input-numlock-label = Açılışta Num Lock açık

# Nerede: Ayarlar uygulamasında input.mouse.sensitivity anahtarının etiketi. Anlam: fare ve dokunmatik yüzeyde işaretçinin hızı.
shell-config-input-sensitivity-label = İşaretçi hızı

# Nerede: Ayarlar uygulamasında input.mouse.accel_profile anahtarının etiketi; seçenekler adaptive (uyarlamalı) ve flat (sabit). Anlam: hızlı hareket edince işaretçinin ivmelenmesi.
shell-config-input-accel-profile-label = İşaretçi ivmesi

# Nerede: Ayarlar uygulamasında `input.mouse.accel_profile` anahtarının "adaptive" seçeneğinin görünen adı. Anlam: işaretçi hızlı hareket edince daha çok yol alır (libinput ivmesi).
shell-schema-input-accel-profile-adaptive = Uyarlamalı

# Nerede: Ayarlar uygulamasında `input.mouse.accel_profile` anahtarının "flat" seçeneğinin görünen adı. Anlam: ivme yok, işaretçi hızı sabit.
shell-schema-input-accel-profile-flat = Sabit

# Nerede: Ayarlar uygulamasında input.mouse.natural_scroll anahtarının etiketi. Anlam: fare tekerleğinde kaydırma yönünün ters çevrilmesi (dokunmatik ekran gibi).
shell-config-input-mouse-natural-scroll-label = Doğal kaydırma (fare)

# Nerede: Ayarlar uygulamasında input.touchpad.natural_scroll anahtarının etiketi. Anlam: dokunmatik yüzeyde kaydırma yönünün ters çevrilmesi.
shell-config-input-touchpad-natural-scroll-label = Doğal kaydırma (dokunmatik yüzey)

# Nerede: Ayarlar uygulamasında input.touchpad.tap_to_click anahtarının etiketi. Anlam: dokunmatik yüzeye hafifçe dokunmak tıklama sayılır.
shell-config-input-tap-to-click-label = Dokunarak tıkla

# Nerede: Ayarlar uygulamasında input.touchpad.disable_while_typing anahtarının etiketi. Anlam: klavyede yazarken dokunmatik yüzey geçici olarak yok sayılır.
shell-config-input-disable-while-typing-label = Yazarken devre dışı bırak

# Nerede: Ayarlar uygulamasında input.touchpad.scroll_factor anahtarının etiketi. Anlam: dokunmatik yüzeyde kaydırma miktarının çarpanı.
shell-config-input-scroll-factor-label = Kaydırma hızı

# Nerede: Ayarlar uygulamasında input.touchpad.scroll_method anahtarının etiketi; seçenekler iki parmak, kenar, düğmeyle, kaydırma yok. Anlam: dokunmatik yüzeyde kaydırmanın nasıl yapıldığı.
shell-config-input-scroll-method-label = Kaydırma yöntemi

# Nerede: Ayarlar uygulamasında `input.touchpad.scroll_method` anahtarının "2fg" seçeneğinin görünen adı. Anlam: dokunmatik yüzeyde iki parmakla kaydırma.
shell-schema-input-scroll-method-2fg = İki parmak

# Nerede: Ayarlar uygulamasında `input.touchpad.scroll_method` anahtarının "edge" seçeneğinin görünen adı. Anlam: dokunmatik yüzeyin kenarında tek parmakla kaydırma.
shell-schema-input-scroll-method-edge = Kenar

# Nerede: Ayarlar uygulamasında `input.touchpad.scroll_method` anahtarının "on_button_down" seçeneğinin görünen adı. Anlam: bir tuş basılı tutulurken işaretçi hareketiyle kaydırma.
shell-schema-input-scroll-method-on-button-down = Tuş basılıyken

# Nerede: Ayarlar uygulamasında `input.touchpad.scroll_method` anahtarının "no_scroll" seçeneğinin görünen adı. Anlam: dokunmatik yüzeyle kaydırma kapalı.
shell-schema-input-scroll-method-no-scroll = Kapalı

# Nerede: shell.toml doğrulama uyarısı (log ve bildirim). Anlam: [input] anahtarına yazılan sayı geçerli aralıkta değil; anahtar yok sayılır, kırpılmaz, kullanıcının Hyprland ayarındaki değer geçerli kalır.
# Değişkenler: $key (ayar anahtarı, örnek: input.keyboard.repeat_rate), $value (yazılan değer, örnek: 500), $range (geçerli aralık, örnek: 1..=100).
shell-config-input-out-of-range = { $key } = { $value } aralık dışı ({ $range }); yok sayıldı, Hyprland ayarındaki değer geçerli

# Nerede: shell.toml doğrulama uyarısı (log ve bildirim). Anlam: [input] anahtarının değeri yanlış türde (ör. sayı yerine metin); anahtar yok sayılır.
# Değişkenler: $key (ayar anahtarı, örnek: input.touchpad.tap_to_click), $value (yazılan değer, örnek: "evet").
shell-config-input-wrong-type = { $key } = { $value } geçersiz türde; yok sayıldı, Hyprland ayarındaki değer geçerli

# Nerede: shell.toml doğrulama uyarısı (log ve bildirim). Anlam: [input] anahtarına bilinmeyen bir seçenek yazılmış; anahtar yok sayılır.
# Değişkenler: $key (ayar anahtarı, örnek: input.mouse.accel_profile), $value (yazılan değer, örnek: "custom"), $options (geçerli seçenekler, örnek: adaptive, flat).
shell-config-input-invalid-choice = { $key } = { $value } geçersiz (geçerli: { $options }); yok sayıldı

# Nerede: shell.toml doğrulama uyarısı (log ve bildirim). Anlam: klavye düzeni ya da seçenek listesinde geçersiz bir öğe var (izin verilmeyen karakter, boş ad); liste bütünüyle yok sayılır.
# Değişkenler: $key (ayar anahtarı, örnek: input.keyboard.layouts), $value (geçersiz öğe, örnek: { layout = "" }).
shell-config-input-invalid-item = { $key }: geçersiz öğe { $value }; yok sayıldı, Hyprland ayarındaki değer geçerli

# Nerede: shell.toml doğrulama uyarısı (log ve bildirim). Anlam: listedeki öğe sayısı izin verilen aralıkta değil; liste yok sayılır.
# Değişkenler: $key (ayar anahtarı, örnek: input.keyboard.layouts), $count (yazılan öğe sayısı, örnek: 5), $range (izin verilen sayı, örnek: 1..=4).
shell-config-input-count = { $key }: { $count } öğe, { $range } olmalı; yok sayıldı

# Nerede: shell.toml doğrulama uyarısı (log ve bildirim). Anlam: [input] bölümünde tanınmayan anahtarlar var ve yok sayıldı.
# Değişkenler: $section (bölüm, örnek: input.keyboard), $keys (anahtar adları, örnek: "nope", "x").
shell-config-input-unknown-key = { $section }: bilinmeyen anahtar { $keys } yok sayıldı

# Nerede: snapshot geçmişi (msg snapshot list/show etiketi; Ayarlar "↺ önceki" listesi). Anlam: bir rolün varsayılan uygulaması değiştirildi (mimeapps.list ya da xdg-terminals.list yazıldı).
# Değişkenler: $role (rol kimliği, örnek: browser, terminal, pdf).
shell-defaults-snapshot-label = Varsayılan uygulama ({ $role })

## Gece ışığı (t113)

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: gece ışığı hızlı ayar kartının görünen adı.
shell-bar-widget-night-light-title = Gece ışığı

# Nerede: Hızlı ayarlarda gece ışığı kartının anahtar satırı başlığı. Anlam: akşam ekran renklerini sıcaklaştıran özellik.
# [max: 24]
shell-quick-night-light-title = Gece ışığı

# Nerede: Gece ışığı anahtar satırının alt metni. Anlam: kapalı, zamanlanmış açılış yok.
shell-quick-night-light-off = Kapalı

# Nerede: Gece ışığı anahtar satırının alt metni. Anlam: açık, zamanlanmış kapanış yok.
shell-quick-night-light-on = Açık

# Nerede: Gece ışığı anahtar satırının alt metni. Anlam: şu an açık, belirtilen saatte kendiliğinden kapanacak.
# Değişkenler: $time (yerel saat, örnek: 07:00).
# [max: 32]
shell-quick-night-light-on-until = Açık · kapanış { $time }

# Nerede: Gece ışığı anahtar satırının alt metni. Anlam: şu an açık, gün doğumunda kendiliğinden kapanacak.
# [max: 32]
shell-quick-night-light-on-until-sunrise = Açık · gün doğumunda kapanır

# Nerede: Gece ışığı anahtar satırının alt metni. Anlam: şu an kapalı, belirtilen saatte kendiliğinden açılacak.
# Değişkenler: $time (yerel saat, örnek: 20:00).
# [max: 32]
shell-quick-night-light-off-until = Kapalı · açılış { $time }

# Nerede: Gece ışığı anahtar satırının alt metni. Anlam: şu an kapalı, gün batımında kendiliğinden açılacak.
# [max: 32]
shell-quick-night-light-off-until-sunset = Kapalı · gün batımında açılır

# Nerede: Gece ışığı anahtar satırının alt metni. Anlam: ekran renklerini başka bir program (ör. kullanıcının kendi hyprsunset'i) yönetiyor, Nuxut dokunmuyor.
# [max: 32]
shell-quick-night-light-blocked = Başka bir uygulama yönetiyor

# Nerede: Gece ışığı sıcaklık kaydırıcısının sağındaki değer. Anlam: renk sıcaklığı Kelvin cinsinden.
# Değişkenler: $kelvin (tam sayı, örnek: 4000).
# [max: 7]
shell-quick-night-light-kelvin = { $kelvin } K

# Nerede: Ayarlar uygulamasında `night_light.schedule` seçeneğinin "always" değeri. Anlam: açıkken gün boyu etkin.
shell-schema-night-light-schedule-always = Her zaman

# Nerede: Ayarlar uygulamasında `night_light.schedule` seçeneğinin "manual" değeri. Anlam: kullanıcının yazdığı saat aralığında etkin.
shell-schema-night-light-schedule-manual = Belirli saatlerde

# Nerede: Ayarlar uygulamasında `night_light.schedule` seçeneğinin "sun" değeri. Anlam: gün batımından gün doğumuna kadar etkin.
shell-schema-night-light-schedule-sun = Gün batımından gün doğumuna

# Nerede: Ayarlar uygulamasında `night_light.enabled` anahtarının etiketi. Anlam: zamanlanmış gece ışığını açar.
shell-config-night-light-enabled-label = Gece ışığı

# Nerede: Ayarlar uygulamasında `night_light.enabled` anahtarının açıklaması. Anlam: hyprsunset başka bir araçtır, ad çevrilmez.
shell-config-night-light-enabled-description = Akşamları ekran renklerini sıcaklaştırır; başka bir gece ışığı aracıyla birlikte açılmaz.

# Nerede: Ayarlar uygulamasında `night_light.temperature` ayarının etiketi. Anlam: gece uygulanan renk sıcaklığı (Kelvin; düşük = daha turuncu).
shell-config-night-light-temperature-label = Gece sıcaklığı

# Nerede: Ayarlar uygulamasında `night_light.schedule` ayarının etiketi. Anlam: gece ışığının ne zaman etkin olacağı.
shell-config-night-light-schedule-label = Zamanlama

# Nerede: Ayarlar uygulamasında `night_light.from` ayarının etiketi. Anlam: belirli saatler zamanlamasında başlangıç saati.
shell-config-night-light-from-label = Başlangıç saati

# Nerede: Ayarlar uygulamasında `night_light.from` ayarının açıklaması. Anlam: bitiş başlangıçtan önceyse ertesi güne geçer.
shell-config-night-light-from-description = Bitiş daha erkense gece yarısını aşar.

# Nerede: Ayarlar uygulamasında `night_light.to` ayarının etiketi. Anlam: belirli saatler zamanlamasında bitiş saati.
shell-config-night-light-to-label = Bitiş saati

# Nerede: Ayarlar uygulamasında `night_light.transition` ayarının etiketi. Anlam: açılış ve kapanışta renklerin yumuşakça değiştiği süre (dakika).
shell-config-night-light-transition-label = Geçiş süresi

# Nerede: shell.toml doğrulama uyarısı (log ve bildirim). Anlam: anahtara yazılan değer geçerli aralıkta değil, kabul edilebilir değer kullanılıyor.
# Değişkenler: $key (ayar anahtarı, örnek: night_light.temperature), $value (yazılan değer, örnek: 900), $fixed (kullanılan değer, örnek: 1500), $range (geçerli aralık, örnek: 1500..=6000).
shell-config-night-light-out-of-range = { $key } = { $value } aralık dışı, { $fixed } kullanılıyor ({ $range })

# Nerede: shell.toml doğrulama uyarısı (log ve bildirim). Anlam: saat SS:DD biçiminde değil, varsayılan saat kullanılıyor.
# Değişkenler: $key (ayar anahtarı, örnek: night_light.from), $value (yazılan değer, örnek: 25:00), $fixed (kullanılan değer, örnek: 20:00).
shell-config-night-light-bad-time = { $key } = "{ $value }" geçersiz saat (SS:DD), { $fixed } kullanılıyor

# Nerede: shell.toml doğrulama uyarısı (log ve bildirim). Anlam: enlem/boylam eksik ya da aralık dışı; konum saat diliminden bulunacak.
shell-config-night-light-bad-location = night_light.latitude/longitude ikisi birlikte ve geçerli aralıkta olmalı; saat dilimindeki konum kullanılıyor

# Nerede: Log uyarısı. Anlam: gün batımı zamanlaması için konum bulunamadı, sabit saatler kullanılıyor.
# Değişkenler: $from (başlangıç, örnek: 20:00), $to (bitiş, örnek: 07:00).
shell-config-night-light-no-location = gün batımı için konum bulunamadı (latitude/longitude yaz ya da saat dilimini ayarla); { $from }–{ $to } kullanılıyor

## Çıkarılabilir medya ([removable_media])

# Nerede: Ayarlar uygulamasında `removable_media.enabled` anahtarının etiketi. Anlam: USB bellek, SD kart gibi çıkarılabilir aygıtların izlenmesi açık mı.
shell-config-removable-media-enabled-label = Çıkarılabilir aygıtlar

# Nerede: Ayarlar uygulamasında `removable_media.enabled` anahtarının altındaki açıklama. Anlam: kapalıyken disk servisine (udisks2) hiç bağlanılmaz; aygıtlar kendiliğinden bağlanmaz, bu sürümde de bağlanmıyor.
shell-config-removable-media-enabled-describe = USB bellek ve hafıza kartlarını izler. Aygıtlar kendiliğinden bağlanmaz; "Aç" deyince bağlanır.

# Nerede: Ayarlar uygulamasında `removable_media.notify` anahtarının etiketi. Anlam: çıkarılabilir aygıt takılınca "Aç / Çıkar" düğmeli bildirim gösterilsin mi.
shell-config-removable-media-notify-label = Takılınca bildir

# Nerede: Çıkarılabilir aygıt takılınca çıkan bildirimin başlığı; gövdede aygıtın adı yazar. Anlam: bir USB bellek, hafıza kartı gibi aygıt bilgisayara takıldı.
# [max: 48]
shell-media-added-title = Çıkarılabilir aygıt takıldı

# Nerede: Çıkarılabilir aygıt bildirimindeki düğme. Anlam: aygıtı bağla ve içeriğini dosya yöneticisinde aç.
# [max: 16]
shell-media-action-open = Aç

# Nerede: Çıkarılabilir aygıt bildirimindeki düğme. Anlam: aygıtı güvenle çıkarmaya hazırla (ayır ve gücünü kes).
# [max: 16]
shell-media-action-eject = Çıkar

# Nerede: Aygıt adı olarak (bildirim gövdesi) etiketi ve modeli bilinmeyen aygıtta. Anlam: belirtilen büyüklükte bir depolama birimi.
# Değişkenler: $size (boyut ve birimi, örnek: 16 GB).
shell-media-size-name = { $size } birim

# Nerede: "Çıkar"dan sonra çıkan bildirimin başlığı. Anlam: aygıt artık bilgisayardan çekilip çıkarılabilir; veri kaybı olmaz.
# [max: 48]
shell-media-ejected-title = Güvenle çıkarılabilir

# Nerede: "Güvenle çıkarılabilir" bildiriminin gövdesi. Anlam: adı verilen aygıt ayrıldı, şimdi çekilebilir.
# Değişkenler: $name (aygıtın adı, örnek: YEDEK).
shell-media-ejected-body = { $name } artık çıkarılabilir.

# Nerede: Aygıt açılamayınca çıkan bildirimin başlığı; gövdede neden yazar. Anlam: aygıt bağlanamadı ya da açılamadı.
# Değişkenler: $name (aygıtın adı, örnek: YEDEK).
shell-media-open-failed-title = { $name } açılamadı

# Nerede: Aygıt çıkarılamayınca çıkan bildirimin başlığı; gövdede neden yazar. Anlam: aygıt ayrılamadı, şimdi çekmek güvenli değil.
# Değişkenler: $name (aygıtın adı, örnek: YEDEK).
shell-media-eject-failed-title = { $name } çıkarılamadı

# Nerede: Aygıt çıkarılamadı bildiriminin gövdesi. Anlam: aygıttaki bir dosya ya da klasör bir uygulamada açık olduğu için ayrılamıyor.
shell-media-error-busy = Aygıttaki dosyalar bir uygulamada açık. Uygulamayı kapatıp yeniden deneyin.

# Nerede: Aygıt açılamadı/çıkarılamadı bildiriminin gövdesi. Anlam: sistem bu işlem için izin vermedi (yetkilendirme reddedildi).
shell-media-error-not-authorized = İzin verilmedi.

# Nerede: Aygıt bildiriminin gövdesi (normalde gösterilmez). Anlam: kullanıcı yetkilendirme penceresini kapatıp işlemden vazgeçti.
shell-media-error-cancelled = İşlemden vazgeçildi.

# Nerede: Aygıt açılamadı/çıkarılamadı bildiriminin gövdesi. Anlam: diskleri yöneten sistem servisi (udisks2) çalışmıyor ya da kurulu değil. "udisks2" paket adıdır, çevrilmez.
shell-media-error-unavailable = Disk servisi (udisks2) çalışmıyor.

# Nerede: Aygıt açılamadı/çıkarılamadı bildiriminin gövdesi. Anlam: aygıt bu arada çekilmiş, artık takılı değil.
shell-media-error-gone = Aygıt artık takılı değil.

# Nerede: Aygıt açılamadı/çıkarılamadı bildiriminin gövdesi (yalnızca geliştirici test örneğinde). Anlam: bu kabuk örneği gerçek aygıtlarda işlem yapmaz.
shell-media-error-disabled = Bu örnek aygıtlarda işlem yapmaz (test örneği).

# Nerede: Aygıt açılamadı/çıkarılamadı bildiriminin gövdesi. Anlam: sistem servisi zamanında yanıt vermedi.
shell-media-error-timeout = Disk servisi yanıt vermedi.

# Nerede: Aygıt açılamadı/çıkarılamadı bildiriminin gövdesi. Anlam: başka bir hata oluştu; sistemin hata metni olduğu gibi eklenir.
# Değişkenler: $detail (sistemin hata metni, örnek: Error mounting: wrong fs type).
shell-media-error-other = Hata: { $detail }

# Nerede: Kabuk güncellendikten sonraki ilk açılışta bir kez çıkan bildirimin başlığı (yanında maskot Fiyu kutlar). Anlam: Nuxut yeni sürüme güncellendi. "Nuxut" ürün adıdır, çevrilmez.
# [max: 60]
shell-update-done-title = Nuxut güncellendi

# Nerede: Aynı güncelleme bildiriminin gövdesi. Anlam: şu an çalışan yeni sürüm numarası.
# Değişkenler: $version (sürüm numarası, örnek: 0.2.0).
shell-update-done-body = Yeni sürüm: { $version }

## Ekran paylaşım seçicisi (nuxut-sharepicker)

# Nerede: Ekran paylaşım seçici kartının başlığı. Anlam: bir uygulama ekran paylaşımı istiyor; kullanıcıdan ekranını paylaşması isteniyor.
# [max: 32]
shell-sharepicker-title = Ekranını paylaş

# Nerede: Kartın birinci sekme etiketi. Anlam: tüm ekran (monitör) paylaşılır.
# [max: 12]
shell-sharepicker-tab-screen = Ekran

# Nerede: Kartın ikinci sekme etiketi. Anlam: tek bir pencere paylaşılır.
# [max: 12]
shell-sharepicker-tab-window = Pencere

# Nerede: Kartın üçüncü sekme etiketi. Anlam: ekranın fareyle seçilen dikdörtgen bölümü paylaşılır.
# [max: 12]
shell-sharepicker-tab-region = Bölge

# Nerede: Kartın birincil düğmesi. Anlam: seçili olan ekran, pencere ya da bölge paylaşılır.
# [max: 16]
shell-sharepicker-share = Paylaş

# Nerede: Kartın ikincil düğmesi. Anlam: vazgeç, hiçbir şey paylaşma.
# [max: 16]
shell-sharepicker-cancel = Vazgeç

# Nerede: Karttaki anahtarlı satırın başlığı. Anlam: bu uygulamaya verilen paylaşım izni hatırlanır.
# [max: 32]
shell-sharepicker-remember = Bu izni hatırla

# Nerede: İzni hatırla satırının altındaki açıklama. Anlam: açarsan uygulama bir sonraki sefer sormadan paylaşmaya başlayabilir; yalnızca güvendiğin uygulamalarda aç.
# [max: 64]
shell-sharepicker-remember-detail = Bir daha sormadan paylaşır; sadece güvendiğin uygulamalarda aç.

# Nerede: Ekran listesinde monitör adının altındaki ayrıntı satırı. Anlam: monitörün çözünürlüğü ve yenileme hızı.
# Değişkenler: $width (piksel, örnek: 2560), $height (piksel, örnek: 1440), $refresh (Hz, tamsayı, örnek: 60).
shell-sharepicker-screen-mode = { $width } × { $height } · { $refresh } Hz

# Nerede: Pencere listesinde, başlığı olmayan pencerenin yer tutucu adı. Anlam: pencere başlığı yok ya da alınamıyor.
# [max: 24]
shell-sharepicker-window-untitled = Adsız pencere

# Nerede: Pencere sekmesi boşken ortada görünen metin. Anlam: paylaşılabilecek açık pencere yok.
# [max: 60]
shell-sharepicker-no-windows = Paylaşılabilecek açık pencere yok.

# Nerede: Pencere sekmesinde, pencere listesi hiç alınamadığında görünen metin. Anlam: liste alınamadı; ekran ya da bölge paylaşılabilir.
# [max: 90]
shell-sharepicker-windows-unavailable = Pencere listesi alınamadı; ekran ya da bölge paylaşabilirsin.

# Nerede: Bölge sekmesinin gövde metni. Anlam: Paylaş düğmesine basıldıktan sonra paylaşılacak alan fareyle sürükleyerek seçilir.
# [max: 110]
shell-sharepicker-region-body = Paylaş düğmesine bastıktan sonra paylaşmak istediğin alanı fareyle sürükleyerek seç.

# Nerede: Bölge seçim katmanının altında (ekranı karartan katmanda) seçim yokken görünen ipucu. Anlam: sürükleyerek bölge seçilir, tıklayınca tüm ekran seçilir, Enter paylaşır, Esc karta geri döner.
# [max: 80]
shell-sharepicker-region-hint = Sürükle: bölge · Tıkla: tüm ekran · Enter: paylaş · Esc: geri

# Nerede: Kartın altındaki kırmızı durum satırı. Anlam: Paylaş düğmesine hiçbir şey seçmeden basıldı; önce bir ekran ya da pencere seçilmeli.
# [max: 60]
shell-sharepicker-pick-first = Önce bir ekran ya da pencere seç.

## Büyüteç (t124)

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: büyüteç hızlı ayar kartının görünen adı.
shell-bar-widget-magnifier-title = Büyüteç

# Nerede: Hızlı ayarlarda büyüteç kartının anahtar satırı başlığı. Anlam: imleç çevresindeki ekranı büyüten erişilebilirlik özelliği.
# [max: 24]
shell-quick-magnifier-title = Büyüteç

# Nerede: Büyüteç anahtar satırının alt metni. Anlam: kapalı, ekran normal boyutta (1×).
shell-quick-magnifier-off = Kapalı

# Nerede: Büyüteç anahtar satırının alt metni. Anlam: açık, şu anki büyütme çarpanıyla çalışıyor.
# Değişkenler: $factor (biçimlenmiş çarpan, örnek: 2,5×).
# [max: 32]
shell-quick-magnifier-on = Açık · { $factor }

# Nerede: Büyüteç çarpan kaydırıcısının sağındaki değer. Anlam: ekranın ne kadar büyütüldüğü.
# Değişkenler: $factor (ondalık sayı, örnek: 2.5).
# [max: 7]
shell-quick-magnifier-factor = { NUMBER($factor, minimumFractionDigits: 1, maximumFractionDigits: 2) }×

# Nerede: Ayarlar uygulamasında keybinds.magnifier_in anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: büyüteçle yakınlaştırma (büyütmeyi artırma) eylemi.
shell-keybinds-magnifier-in-label = Büyüteç: yakınlaştır

# Nerede: Ayarlar uygulamasında keybinds.magnifier_out anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: büyüteçle uzaklaştırma (büyütmeyi azaltma) eylemi.
shell-keybinds-magnifier-out-label = Büyüteç: uzaklaştır

# Nerede: Ayarlar uygulamasında keybinds.magnifier_reset anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: büyütmeyi kapatma (1×'e döndürme) eylemi.
shell-keybinds-magnifier-reset-label = Büyüteç: kapat

## Temel katman kısayolları (ADR-0006, docs/design/hyprland-base.md)

# Nerede: Ayarlar uygulamasında keybinds.launcher anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: Super tuşuna tek başına basıp bırakınca uygulama başlatıcısını açıp kapatma eylemi (başka tuşla birlikte basılırsa çalışmaz).
shell-keybinds-launcher-label = Uygulama başlatıcısı

# Nerede: Ayarlar uygulamasında keybinds.lock anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: ekranı kilitleme eylemi.
shell-keybinds-lock-label = Ekranı kilitle

# Nerede: Ayarlar uygulamasında keybinds.session_menu anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: oturum eylemlerini (kilitle, oturumu kapat, yeniden başlat, kapat) içeren paneli açma eylemi.
shell-keybinds-session-menu-label = Oturum menüsü

# Nerede: Ayarlar uygulamasında keybinds.settings anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: Nuxut Ayarlar uygulamasını açma eylemi.
shell-keybinds-settings-label = Ayarlar

# Nerede: Ayarlar uygulamasında keybinds.terminal anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: varsayılan terminal uygulamasını açma eylemi.
shell-keybinds-terminal-label = Terminal

# Nerede: Ayarlar uygulamasında keybinds.file_manager anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: varsayılan dosya yöneticisini açma eylemi.
shell-keybinds-file-manager-label = Dosya yöneticisi

# Nerede: Ayarlar uygulamasında keybinds.browser anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: varsayılan web tarayıcısını açma eylemi.
shell-keybinds-browser-label = Web tarayıcısı

# Nerede: Ayarlar uygulamasında keybinds.close_window anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: etkin pencereyi kapatma eylemi.
shell-keybinds-close-window-label = Pencereyi kapat

# Nerede: Ayarlar uygulamasında keybinds.close_window_alt anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: etkin pencereyi kapatmanın ikinci kısayolu (genellikle Alt+F4).
shell-keybinds-close-window-alt-label = Pencereyi kapat (ikinci kısayol)
# Nerede: Ayarlar uygulamasında keybinds.force_kill anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: yanıt vermeyen bir pencereyi zorla kapatma; kısayoldan sonra kapatılacak pencereye tıklanır.
shell-keybinds-force-kill-label = Yanıt vermeyen pencereyi zorla kapat

# Nerede: Ayarlar uygulamasında keybinds.fullscreen anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: etkin pencereyi tam ekran yapma ya da tam ekrandan çıkarma eylemi.
shell-keybinds-fullscreen-label = Tam ekran

# Nerede: Ayarlar uygulamasında keybinds.maximize anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: etkin pencereyi ekranı kaplayacak biçimde büyütme ya da eski boyutuna döndürme eylemi (bar görünür kalır).
shell-keybinds-maximize-label = Ekranı kapla

# Nerede: Ayarlar uygulamasında keybinds.toggle_floating anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: etkin pencereyi yüzen pencere yapma ya da döşemeye geri alma eylemi.
shell-keybinds-toggle-floating-label = Yüzen pencere

# Nerede: Ayarlar uygulamasında keybinds.pin anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: yüzen pencereyi her çalışma alanında görünür tutma (sabitleme) eylemi.
shell-keybinds-pin-label = Pencereyi sabitle

# Nerede: Ayarlar uygulamasında keybinds.focus_left anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: odağı soldaki pencereye taşıma eylemi.
shell-keybinds-focus-left-label = Odak: sol

# Nerede: Ayarlar uygulamasında keybinds.focus_right anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: odağı sağdaki pencereye taşıma eylemi.
shell-keybinds-focus-right-label = Odak: sağ

# Nerede: Ayarlar uygulamasında keybinds.focus_up anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: odağı üstteki pencereye taşıma eylemi.
shell-keybinds-focus-up-label = Odak: yukarı

# Nerede: Ayarlar uygulamasında keybinds.focus_down anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: odağı alttaki pencereye taşıma eylemi.
shell-keybinds-focus-down-label = Odak: aşağı

# Nerede: Ayarlar uygulamasında keybinds.move_left anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: etkin pencereyi sola taşıma eylemi.
shell-keybinds-move-left-label = Pencereyi sola taşı

# Nerede: Ayarlar uygulamasında keybinds.move_right anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: etkin pencereyi sağa taşıma eylemi.
shell-keybinds-move-right-label = Pencereyi sağa taşı

# Nerede: Ayarlar uygulamasında keybinds.move_up anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: etkin pencereyi yukarı taşıma eylemi.
shell-keybinds-move-up-label = Pencereyi yukarı taşı

# Nerede: Ayarlar uygulamasında keybinds.move_down anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: etkin pencereyi aşağı taşıma eylemi.
shell-keybinds-move-down-label = Pencereyi aşağı taşı

# Nerede: Ayarlar uygulamasında keybinds.shrink_width anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: etkin pencerenin genişliğini azaltma eylemi (basılı tutunca sürer).
shell-keybinds-shrink-width-label = Pencereyi daralt

# Nerede: Ayarlar uygulamasında keybinds.grow_width anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: etkin pencerenin genişliğini artırma eylemi (basılı tutunca sürer).
shell-keybinds-grow-width-label = Pencereyi genişlet

# Nerede: Ayarlar uygulamasında keybinds.shrink_height anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: etkin pencerenin yüksekliğini azaltma eylemi (basılı tutunca sürer).
shell-keybinds-shrink-height-label = Pencereyi kısalt

# Nerede: Ayarlar uygulamasında keybinds.grow_height anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: etkin pencerenin yüksekliğini artırma eylemi (basılı tutunca sürer).
shell-keybinds-grow-height-label = Pencereyi uzat

# Nerede: Ayarlar uygulamasında keybinds.workspace anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: değiştirici tuşla birlikte rakam satırındaki 1…0 tuşlarına basınca o numaralı çalışma alanına geçme eylemi; değer yalnızca değiştiricidir.
shell-keybinds-workspace-label = Çalışma alanına geç (1…0)

# Nerede: Ayarlar uygulamasında keybinds.move_to_workspace anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: değiştirici tuşla birlikte 1…0 tuşlarına basınca etkin pencereyi o numaralı çalışma alanına taşıma eylemi.
shell-keybinds-move-to-workspace-label = Pencereyi çalışma alanına taşı (1…0)

# Nerede: Ayarlar uygulamasında keybinds.workspace_next anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: bir sonraki numaralı çalışma alanına geçme eylemi.
shell-keybinds-workspace-next-label = Sonraki çalışma alanı

# Nerede: Ayarlar uygulamasında keybinds.workspace_prev anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: bir önceki numaralı çalışma alanına geçme eylemi.
shell-keybinds-workspace-prev-label = Önceki çalışma alanı

# Nerede: Ayarlar uygulamasında keybinds.move_to_workspace_next anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: etkin pencereyi bir sonraki çalışma alanına taşıma eylemi.
shell-keybinds-move-to-workspace-next-label = Pencereyi sonraki alana taşı

# Nerede: Ayarlar uygulamasında keybinds.move_to_workspace_prev anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: etkin pencereyi bir önceki çalışma alanına taşıma eylemi.
shell-keybinds-move-to-workspace-prev-label = Pencereyi önceki alana taşı

# Nerede: Ayarlar uygulamasında keybinds.workspace_last anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: en son kullanılan çalışma alanına dönme eylemi.
shell-keybinds-workspace-last-label = Son çalışma alanına dön

# Nerede: Ayarlar uygulamasında keybinds.cycle_windows anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: çalışma alanındaki pencereler arasında sırayla bir sonrakine geçme eylemi.
shell-keybinds-cycle-windows-label = Pencereler arasında geçiş

# Nerede: Ayarlar uygulamasında keybinds.cycle_windows_back anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: çalışma alanındaki pencereler arasında ters sırayla geçme eylemi.
shell-keybinds-cycle-windows-back-label = Pencereler arasında geri geçiş

# Nerede: Ayarlar uygulamasında keybinds.scratchpad anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: gizli karalama çalışma alanını gösterme ya da gizleme eylemi (her an çağrılabilen pencereler için).
shell-keybinds-scratchpad-label = Karalama alanını göster/gizle

# Nerede: Ayarlar uygulamasında keybinds.move_to_scratchpad anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: etkin pencereyi gizli karalama çalışma alanına gönderme eylemi.
shell-keybinds-move-to-scratchpad-label = Pencereyi karalama alanına gönder

# Nerede: Ayarlar uygulamasında keybinds.toggle_split anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: döşenmiş pencerelerin bölünme yönünü (yan yana / alt alta) çevirme eylemi.
shell-keybinds-toggle-split-label = Bölme yönünü çevir

# Nerede: Ayarlar uygulamasında keybinds.window_drag anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: değiştirici tuş basılıyken farenin sol düğmesiyle pencereyi sürükleyip taşıma eylemi.
shell-keybinds-window-drag-label = Fareyle pencere taşı

# Nerede: Ayarlar uygulamasında keybinds.window_resize_drag anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: değiştirici tuş basılıyken farenin sağ düğmesiyle sürükleyerek pencereyi boyutlandırma eylemi.
shell-keybinds-window-resize-drag-label = Fareyle pencere boyutlandır

# Nerede: Ayarlar uygulamasında keybinds.workspace_scroll_next anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: değiştirici tuş basılıyken fare tekerleğini aşağı çevirince bir sonraki açık çalışma alanına geçme eylemi.
shell-keybinds-workspace-scroll-next-label = Tekerlekle sonraki çalışma alanı

# Nerede: Ayarlar uygulamasında keybinds.workspace_scroll_prev anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: değiştirici tuş basılıyken fare tekerleğini yukarı çevirince bir önceki açık çalışma alanına geçme eylemi.
shell-keybinds-workspace-scroll-prev-label = Tekerlekle önceki çalışma alanı

# Nerede: Ayarlar uygulamasında `magnifier.step` satırı. Anlam: yakınlaştır/uzaklaştır tuşunun her basışta büyütmeyi çarptığı/böldüğü adım (örnek 1,25).
shell-config-magnifier-step-label = Adım

# Nerede: Ayarlar uygulamasında `magnifier.step` satırının açıklaması. Anlam: her yakınlaştırma büyütmeyi bu sayıyla çarpar, uzaklaştırma böler.
shell-config-magnifier-step-describe = Her yakınlaştırma büyütmeyi bu sayıyla çarpar, uzaklaştırma böler.

# Nerede: Ayarlar uygulamasında `magnifier.max` satırı. Anlam: en büyük büyütme çarpanı.
shell-config-magnifier-max-label = En büyük çarpan

# Nerede: Ayarlar uygulamasında `magnifier.factor` satırı. Anlam: büyüteç kapalıyken açılınca kullanılan çarpan.
shell-config-magnifier-factor-label = Açılış çarpanı

# Nerede: Ayarlar uygulamasında `magnifier.factor` satırının açıklaması. Anlam: bu oturumda daha önce başka bir çarpan kullanıldıysa o kullanılır.
shell-config-magnifier-factor-describe = Bu oturumda daha önce başka bir çarpan kullanıldıysa o kullanılır.

# Nerede: Config uyarısı (log). Anlam: yazılan değer geçerli aralığın dışında, onun yerine düzeltilmiş değer kullanılıyor.
# Değişkenler: $key (ayar anahtarı, örnek: magnifier.step), $value (yazılan değer), $fixed (kullanılan değer), $range (geçerli aralık, örnek: 1.05..=2).
shell-config-magnifier-out-of-range = { $key } = { $value } aralık dışı, { $fixed } kullanılıyor ({ $range })

## Hızlı anahtarlar kartı (toggles)

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: kenar panelindeki tek dokunuşluk anahtar ızgarası kartının görünen adı.
shell-bar-widget-toggles-title = Hızlı anahtarlar

# Nerede: Hızlı anahtarlar kartında kutucuk etiketi. Anlam: rahatsız etme kipi (bildirim popup'ları ve sesleri susturulur).
# [max: 18]
shell-quick-toggle-dnd = Rahatsız etme

# Nerede: Hızlı anahtarlar kartında "Rahatsız etme" kutucuğunun açıkken alt metni. Anlam: bildirimler sessiz, merkezde birikir.
# [max: 22]
shell-quick-toggle-dnd-on = Bildirimler sessizde

# Nerede: Hızlı anahtarlar kartında kutucuk etiketi. Anlam: ekranın kararmasını, kilitlenmesini ve uykuyu geçici olarak engeller ("kahve modu").
# [max: 18]
shell-quick-toggle-caffeine = Uyanık tut

# Nerede: Hızlı anahtarlar kartında "Uyanık tut" kutucuğunun açıkken alt metni. Anlam: ekran kararmaz, kilitlenmez, makine uyumaz.
# [max: 22]
shell-quick-toggle-caffeine-on = Ekran açık kalır

# Nerede: Hızlı anahtarlar kartında kutucuk etiketi. Anlam: koyu renk düzeni; kapalıyken açık tema.
# [max: 18]
shell-quick-toggle-dark = Koyu tema

# Nerede: Hızlı anahtarlar kartında "Koyu tema" kutucuğunun açıkken alt metni. Anlam: anahtar açık.
# [max: 22]
shell-quick-toggle-on = Açık
## Ekran klavyesi (t125)

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: ekran klavyesi hızlı ayar kartının görünen adı.
shell-bar-widget-osk-title = Ekran klavyesi

# Nerede: Hızlı ayarlarda ekran klavyesi kartının anahtar satırı başlığı. Anlam: fare ya da dokunmayla yazmak için ekranda gösterilen klavye.
# [max: 24]
shell-quick-osk-title = Ekran klavyesi

# Nerede: Ekran klavyesi anahtar satırının alt metni. Anlam: klavye gizli.
shell-quick-osk-off = Kapalı

# Nerede: Ekran klavyesi anahtar satırının alt metni. Anlam: klavye ekranda, şu düzenle yazıyor.
# Değişkenler: $layout (düzen adı, örnek: Türkçe Q).
# [max: 32]
shell-quick-osk-on = Açık · { $layout }

# Nerede: Ekran klavyesi kartının alt metninde ve Ayarlar'daki düzen seçiminde. Anlam: Türkçe Q klavye düzeni.
shell-quick-osk-layout-tr = Türkçe Q

# Nerede: Ekran klavyesi kartının alt metninde ve Ayarlar'daki düzen seçiminde. Anlam: ABD İngilizcesi klavye düzeni.
shell-quick-osk-layout-us = İngilizce (ABD)

# Nerede: Ekran klavyesinin alt satırındaki Control tuşunun üstündeki yazı. Anlam: klavyedeki Ctrl tuşunun adı; kısa tutulmalı.
# [max: 5]
shell-osk-key-ctrl = Ctrl

# Nerede: Ekran klavyesinin alt satırındaki AltGr tuşunun üstündeki yazı. Anlam: üçüncü seviye karakterleri (@, €, ₺) yazdıran AltGr tuşunun adı.
# [max: 6]
shell-osk-key-altgr = AltGr

# Nerede: Ayarlar uygulamasında keybinds.osk_toggle anahtarının satır etiketi; atanan kısayol bu eylemin adının yanında gösterilir. Anlam: ekran klavyesini gösterme/gizleme eylemi.
shell-keybinds-osk-toggle-label = Ekran klavyesi: aç/kapat

# Nerede: Ayarlar uygulamasında `osk.layout` satırı. Anlam: ekran klavyesinin açılıştaki düzeni.
shell-config-osk-layout-label = Düzen

# Nerede: Ayarlar uygulamasında `osk.layout` satırının açıklaması. Anlam: "Otomatik" seçiliyse ekran klavyesi, sistemde o an etkin olan klavye düzenini izler (açıklamada teknik ad yok).
shell-config-osk-layout-describe = Otomatik: etkin klavye düzenini izler.

# Nerede: Ayarlar uygulamasında `osk.layout` seçeneği. Anlam: düzen kendiliğinden seçilir.
shell-schema-osk-layout-auto = Otomatik

# Nerede: Ayarlar uygulamasında `osk.layout` seçeneği. Anlam: Türkçe Q klavye düzeni.
shell-schema-osk-layout-tr = Türkçe Q

# Nerede: Ayarlar uygulamasında `osk.layout` seçeneği. Anlam: ABD İngilizcesi klavye düzeni.
shell-schema-osk-layout-us = İngilizce (ABD)

# Nerede: Ayarlar uygulamasında `osk.key_height` satırı. Anlam: ekran klavyesindeki tuşların yüksekliği (piksel); klavyenin boyu bundan türer.
shell-config-osk-key-height-label = Tuş yüksekliği

# Nerede: Config uyarısı (log). Anlam: yazılan değer geçerli aralığın dışında, onun yerine düzeltilmiş değer kullanılıyor.
# Değişkenler: $key (ayar anahtarı, örnek: osk.key_height), $value (yazılan değer), $fixed (kullanılan değer), $range (geçerli aralık, örnek: 32..=72).
shell-config-osk-out-of-range = { $key } = { $value } aralık dışı, { $fixed } kullanılıyor ({ $range })

# Nerede: Ekran okuyucunun ekran klavyesindeki tuş için söylediği ad (erişilebilir ad; ekranda görünmez). Anlam: imlecin solundaki karakteri silen tuş.
shell-osk-key-backspace = Geri sil

# Nerede: Ekran okuyucunun ekran klavyesindeki tuş için söylediği ad. Anlam: Tab (sekme) tuşu.
shell-osk-key-tab = Sekme

# Nerede: Ekran okuyucunun ekran klavyesindeki tuş için söylediği ad. Anlam: Enter tuşu (satırı onaylar / yeni satır).
shell-osk-key-enter = Enter

# Nerede: Ekran okuyucunun ekran klavyesindeki tuş için söylediği ad. Anlam: boşluk tuşu.
shell-osk-key-space = Boşluk

# Nerede: Ekran okuyucunun ekran klavyesindeki tuş için söylediği ad. Anlam: sol ok tuşu.
shell-osk-key-left = Sol ok

# Nerede: Ekran okuyucunun ekran klavyesindeki tuş için söylediği ad. Anlam: sağ ok tuşu.
shell-osk-key-right = Sağ ok

# Nerede: Ekran okuyucunun ekran klavyesindeki tuş için söylediği ad. Anlam: yukarı ok tuşu.
shell-osk-key-up = Yukarı ok

# Nerede: Ekran okuyucunun ekran klavyesindeki tuş için söylediği ad. Anlam: aşağı ok tuşu.
shell-osk-key-down = Aşağı ok

# Nerede: Ekran okuyucunun ekran klavyesindeki tuş için söylediği ad. Anlam: bir sonraki harfi büyük yazdıran Shift tuşu.
shell-osk-key-shift = Shift

# Nerede: Ekran okuyucunun ekran klavyesindeki tuş için söylediği ad. Anlam: Caps Lock (büyük harf kilidi) tuşu.
shell-osk-key-caps = Büyük harf kilidi

# Nerede: Ekran okuyucunun ekran klavyesindeki düzen tuşu için söylediği ad. Anlam: ekran klavyesinin düzenini (TR/US) değiştirir; şu anki düzen söylenir.
# Değişkenler: $layout (şu anki düzenin kısaltması, örnek: TR).
shell-osk-key-switch-layout = Düzeni değiştir, şu an { $layout }

# Nerede: Ekran okuyucunun ekran klavyesindeki tuş için söylediği ad. Anlam: ekran klavyesini gizleyen tuş.
shell-osk-key-hide = Klavyeyi gizle


## Dosya seçici (nuxut-filechooser; docs/design/file-chooser.md)

# Nerede: Dosya seçici penceresinin başlığı (uygulama başlık vermediyse), dosya açarken.
# [max: 40]
shell-filechooser-title-open = Dosya aç

# Nerede: Dosya seçici penceresinin başlığı (uygulama başlık vermediyse), klasör seçerken.
# [max: 40]
shell-filechooser-title-folder = Klasör seç

# Nerede: Dosya seçici penceresinin başlığı (uygulama başlık vermediyse), kaydederken.
# [max: 40]
shell-filechooser-title-save = Kaydet

# Nerede: Dosya seçicinin kabul düğmesi, dosya açarken.
# [max: 16]
shell-filechooser-open = Aç

# Nerede: Dosya seçicinin kabul düğmesi, klasör seçerken.
# [max: 16]
shell-filechooser-select = Seç

# Nerede: Dosya seçicinin kabul düğmesi, kaydederken.
# [max: 16]
shell-filechooser-save = Kaydet

# Nerede: Dosya seçicide ve onay diyaloglarında vazgeçme düğmesi.
# [max: 16]
shell-filechooser-cancel = Vazgeç

# Nerede: Dosya seçicinin üstündeki arama alanının yer tutucusu. Anlam: bulunulan klasörde ada göre süzer.
# [max: 24]
shell-filechooser-search = Bu klasörde ara

# Nerede: Ctrl+L ile açılan konum alanının yer tutucusu. Anlam: kullanıcı bir yol yazıp Enter'a basar.
# [max: 40]
shell-filechooser-location-placeholder = Konum yazın (ör. ~/Belgeler)

# Nerede: Kaydederken dosya adı alanının solundaki etiket.
# [max: 12]
shell-filechooser-name = Ad

# Nerede: Kaydederken dosya adı alanının yer tutucusu.
# [max: 24]
shell-filechooser-name-placeholder = Dosya adı

# Nerede: Filtre seçiminde uygulamanın adsız verdiği filtre.
# [max: 24]
shell-filechooser-filter-unnamed = Dosyalar

# Nerede: Kenar çubuğunda ev dizini ve başlıkta ev dizinindeyken.
# [max: 20]
shell-filechooser-place-home = Ev

# Nerede: Kenar çubuğunda masaüstü klasörü.
# [max: 20]
shell-filechooser-place-desktop = Masaüstü

# Nerede: Kenar çubuğunda belgeler klasörü.
# [max: 20]
shell-filechooser-place-documents = Belgeler

# Nerede: Kenar çubuğunda indirilenler klasörü.
# [max: 20]
shell-filechooser-place-downloads = İndirilenler

# Nerede: Kenar çubuğunda resimler klasörü.
# [max: 20]
shell-filechooser-place-pictures = Resimler

# Nerede: Kenar çubuğunda müzik klasörü.
# [max: 20]
shell-filechooser-place-music = Müzik

# Nerede: Kenar çubuğunda videolar klasörü.
# [max: 20]
shell-filechooser-place-videos = Videolar

# Nerede: Kenar çubuğunda kök dizin (/). Anlam: tüm dosya sistemi.
# [max: 20]
shell-filechooser-place-root = Bilgisayar

# Nerede: Kenar çubuğunda ilk grubun başlığı.
# [max: 20]
shell-filechooser-group-places = Yerler

# Nerede: Kenar çubuğunda takılı/çıkarılabilir aygıtlar grubunun başlığı.
# [max: 20]
shell-filechooser-group-devices = Aygıtlar

# Nerede: Kenar çubuğunda yer imleri grubunun başlığı.
# [max: 20]
shell-filechooser-group-bookmarks = Yer imleri

# Nerede: Bağlı olmayan bir aygıta tıklanınca durum satırı. Anlam: aygıt bağlanıyor, bitince açılacak.
# [max: 40]
shell-filechooser-mounting = Aygıt bağlanıyor…

# Nerede: Çok büyük bir klasörde durum satırı. Anlam: yalnızca ilk girdiler listelendi.
# [max: 60]
shell-filechooser-truncated = Klasör çok büyük; ilk 50.000 öğe gösteriliyor.

# Nerede: Boş klasörde liste alanının ortasında.
# [max: 40]
shell-filechooser-empty = Bu klasör boş

# Nerede: Aramada eşleşme yokken liste alanının ortasında.
# [max: 40]
shell-filechooser-no-results = Eşleşen dosya yok

# Nerede: Okuma izni olmayan klasörde liste alanının ortasında; ya da klasör oluşturulamadığında durum satırında.
# [max: 48]
shell-filechooser-error-denied = Bu konuma erişim izniniz yok

# Nerede: Var olmayan klasöre gidilince liste alanının ortasında.
# [max: 40]
shell-filechooser-error-missing = Bu konum bulunamadı

# Nerede: Klasör başka bir nedenle okunamadığında liste alanının ortasında.
# [max: 40]
shell-filechooser-error-read = Klasör okunamadı

# Nerede: Yeni klasör oluşturulamadığında durum satırı.
# [max: 40]
shell-filechooser-error-create = Klasör oluşturulamadı

# Nerede: Aynı adda öğe varken yeni klasör oluşturulmak istenince durum satırı.
# [max: 40]
shell-filechooser-error-exists = Bu adda bir öğe zaten var

# Nerede: Kaydederken var olan dosya seçilince açılan onay diyaloğunun başlığı.
# [max: 32]
shell-filechooser-overwrite-title = Dosya değiştirilsin mi?

# Nerede: Üzerine yazma onay diyaloğunun gövdesi. { $name } dosya adıdır.
# [max: 120]
shell-filechooser-overwrite-body = “{ $name }” zaten var. Kaydederseniz içeriği değiştirilecek.

# Nerede: Üzerine yazma onay diyaloğunun yıkıcı eylem düğmesi.
# [max: 16]
shell-filechooser-overwrite = Değiştir

# Nerede: Yeni klasör diyaloğunun başlığı ve ipucu.
# [max: 24]
shell-filechooser-new-folder = Yeni klasör

# Nerede: Yeni klasör diyaloğundaki ad alanının yer tutucusu.
# [max: 24]
shell-filechooser-new-folder-placeholder = Klasör adı

# Nerede: Yeni klasör diyaloğunun onay düğmesi.
# [max: 16]
shell-filechooser-create = Oluştur

# Nerede: Kaydetme adı boşken durum satırı.
# [max: 48]
shell-filechooser-name-empty = Bir ad yazın

# Nerede: Ad "." ya da ".." iken durum satırı.
# [max: 48]
shell-filechooser-name-dot = “.” ve “..” ad olarak kullanılamaz

# Nerede: Adda "/" varken durum satırı.
# [max: 48]
shell-filechooser-name-slash = Ad “/” içeremez

# Nerede: Ad 255 bayttan uzunken durum satırı.
# [max: 48]
shell-filechooser-name-long = Ad çok uzun
# Nerede: Ayarlar uygulamasında `osk.reserve_space` anahtarı (Erişilebilirlik › Ekran klavyesi). Anlam: alta yanaşık ekran klavyesi için ekranın altında yer ayrılsın, pencereler klavyenin üstüne itilsin.
shell-config-osk-reserve-space-label = Pencereleri klavyenin üstüne it

# Nerede: Ayarlar uygulamasında `osk.reserve_space` satırının açıklaması. Anlam: kapalıyken klavye pencerelerin üstünde durur ve yazı alanını örtebilir; açıkken pencereler küçülür.
shell-config-osk-reserve-space-describe = Kapalıyken klavye pencerelerin üstünde durur ve yazdığın alanı örtebilir.

## Ses karıştırıcı kartı (mixer)

# Nerede: Ayarlar uygulamasının widget listesi ("widget ekle") ve widget ayar sayfaları. Anlam: ses çalan uygulamaların seviyesini ve sessizini ayrı ayrı ayarlayan panel kartının görünen adı.
shell-bar-widget-mixer-title = Ses karıştırıcı

## Kilit ekranı başlatma hataları (kabuk)

# Nerede: kilitleme başarısız olunca açılan bildirimin başlığı. Anlam: Kilitle düğmesi/kısayolu ya da otomatik kilit ekranı kilitleyemedi.
shell-lock-failed-title = Ekran kilitlenemedi

# Nerede: kilit bildirimi gövdesi. Anlam: kilit ekranı programı kabuğun yanında yok; $path beklenen dosya yolu.
shell-lock-missing-body = Kilit ekranı programı yok, Kilitle çalışmaz. Nuxut'u yeniden kurun (install.sh). Beklenen yer: { $path }

# Nerede: kilit bildirimi gövdesi. Anlam: kilit ekranı programı çalıştırılamadı; $error teknik neden.
shell-lock-spawn-body = Kilit ekranı başlatılamadı: { $error }

# Nerede: kilit bildirimi gövdesi. Anlam: compositor ikinci bir kilit ekranına izin vermedi; kullanıcının kendi kilitleyicisi (ör. hyprlock) olabilir.
shell-lock-refused-body = Başka bir kilit ekranı etkin olabilir. Başka bir kilit ekranı kullanılacaksa Ayarlar › Güç › Kilit › "Nuxut kilit ekranını kullan" kapatılabilir.

# Nerede: kilit bildirimi gövdesi. Anlam: kilit ekranı programı hatayla kapandı; $reason çıkış kodu ya da sinyal (teknik).
shell-lock-crashed-body = Kilit ekranı beklenmedik biçimde kapandı ({ $reason }). Ayrıntı: journalctl --user -u nuxut-shell

# Nerede: kabuk açılışında kilit ekranı programı yoksa çıkan bildirimin başlığı. Anlam: Kilitle hiçbir şey açmayacak; gövde shell-lock-missing-body.
shell-lock-missing-title = Kilit ekranı kurulu değil

# Nerede: kilit ekranı reddedildi bildiriminin düğmesi. Anlam: Ayarlar uygulamasını Güç sayfasında açar.
# [max: 20]
shell-lock-action-settings = Ayarları aç


## Durum raporu (nuxut-shell doctor; docs/design/doctor.md). Terminalde düz metin; satır başında ✓/⚠/✗.

# Nerede: Raporun ilk satırı. Anlam: Nuxut'un tanı raporu ve kurulu sürüm. Değişkenler: $version.
shell-doctor-title = Nuxut durum raporu (nuxut-shell { $version })

# Nerede: Raporun son satırı. Anlam: kaç denetim geçti, kaçı uyarı, kaçı hata. Değişkenler: $ok, $warn, $fail.
shell-doctor-summary = Özet: { $ok } ✓ · { $warn } ⚠ · { $fail } ✗

# Nerede: Sürüm ya da commit bilinmediğinde yerine yazılır.
shell-doctor-unknown = bilinmiyor

# Nerede: Kabuk çalışmadığı için canlı bilgi gerektiren bir denetim yapılamadığında.
shell-doctor-needs-shell = Kabuk çalışmadığı için denetlenemedi

# Nerede: Bölüm başlığı. Anlam: ikililer, PATH, sürüm.
shell-doctor-section-install = Kurulum

# Nerede: Bölüm başlığı. Anlam: çalışan kabuk süreci, nasıl başlatıldığı, oturumu.
shell-doctor-section-shell = Kabuk

# Nerede: Bölüm başlığı.
shell-doctor-section-hyprland = Hyprland

# Nerede: Bölüm başlığı. Anlam: birleşik görünüm (GTK/Qt teması, ikon, imleç).
shell-doctor-section-look = Görünüm

# Nerede: Bölüm başlığı.
shell-doctor-section-audio = Ses

# Nerede: Bölüm başlığı. Anlam: xdg-desktop-portal arka ucu, dosya seçici, ekran paylaşım seçicisi.
shell-doctor-section-portal = Portal

# Nerede: Bölüm başlığı. Anlam: kilit ekranı.
shell-doctor-section-lock = Kilit

# Nerede: Bölüm başlığı. Anlam: pano geçmişi ve sanal makine pano köprüsü.
shell-doctor-section-clipboard = Pano

# Nerede: Pano bölümü. Anlam: pano geçmişi açık ve kabuk panoyu izliyor.
shell-doctor-clipboard-history-ok = Pano geçmişi açık ve izleniyor

# Nerede: Pano bölümü. Anlam: kullanıcı pano geçmişini kapatmış (bilgi).
shell-doctor-clipboard-history-off = Pano geçmişi kapalı (shell.toml [clipboard] enabled)

# Nerede: Pano bölümü. Anlam: compositor pano izleme protokolünü (data-control) sunmuyor; geçmiş ve köprü çalışmaz.
shell-doctor-clipboard-no-protocol = Compositor pano protokolü (data-control) sunmuyor: pano geçmişi ve VM köprüsü çalışmaz

# Nerede: Öneri satırı. Anlam: data-control sunan bir Hyprland sürümüne güncellemek gerekir.
shell-doctor-hint-clipboard-no-protocol = Hyprland'i güncelleyin; pano protokolünü (data-control) sunan bir sürüm gerekir

# Nerede: Pano bölümü. Anlam: pano izlemesi durmuş, kabuk kendiliğinden yeniden deniyor.
shell-doctor-clipboard-stopped = Pano izlemesi durdu; kabuk yeniden deniyor

# Nerede: Öneri satırı. Anlam: ayrıntı için kabuğun günlüğüne bakılır (komut çevrilmez).
shell-doctor-hint-clipboard-journal = Ayrıntı: journalctl --user -t nuxut-shell | grep pano

# Nerede: Pano bölümü. Anlam: çalışan kabuk eski; pano durumunu bildirmiyor (bilgi).
shell-doctor-clipboard-unknown = Kabuk pano durumunu bildirmiyor (eski sürüm)

# Nerede: Pano bölümü. Anlam: sanal makine pano köprüsü ayarla kapatılmış (bilgi).
shell-doctor-clipboard-bridge-off = VM pano köprüsü kapalı ([clipboard] x11_bridge = "off")

# Nerede: Pano bölümü. Anlam: sanal makine pano köprüsü çalışıyor; $mode ayar değeri (auto/on, çevrilmez).
shell-doctor-clipboard-bridge-active = VM pano köprüsü devrede (x11_bridge = { $mode })

# Nerede: Pano bölümü. Anlam: köprü kurulu ama sanal makine konuk ajanı bulunmadı; gerçek makinede normaldir.
shell-doctor-clipboard-bridge-waiting = VM pano köprüsü beklemede: konuk ajanı (spice-vdagent vb.) çalışmıyor

# Nerede: Pano bölümü. Anlam: köprü açık ayarlı ama çalışmıyor; $mode ayar değeri (auto/on, çevrilmez).
shell-doctor-clipboard-bridge-down = VM pano köprüsü çalışmıyor (x11_bridge = { $mode })

# Nerede: Pano bölümü. Anlam: Nuxut'la aynı işi yapan harici pano aracı bulunmadı.
shell-doctor-clipboard-tools-none = Harici pano eşitleyicisi ya da geçmiş yöneticisi yok

# Nerede: Pano bölümü. Anlam: X11 ↔ Wayland panosunu eşitleyen harici betik/araç bulundu; $list araç adları ve dosya:satır.
shell-doctor-clipboard-sync = Harici pano eşitleyicisi: { $list }

# Nerede: Öneri satırı. Anlam: iki köprü birbirini besler; harici satırı yorum yapıp oturumu yeniden başlatmak gerekir.
shell-doctor-hint-clipboard-sync = Nuxut'un VM köprüsü aynı işi yapar; o satırın başına # koyup oturumu yeniden başlatın (geri almak için # silinir)

# Nerede: Pano bölümü. Anlam: başka bir pano geçmişi programı bulundu; $list araç adları ve dosya:satır.
shell-doctor-clipboard-managers = Başka pano geçmişi yöneticisi: { $list }

# Nerede: Öneri satırı. Anlam: iki pano geçmişi aynı anda çalışmasın; birini kapatmak gerekir.
shell-doctor-hint-clipboard-managers = Nuxut pano geçmişi de açık; o satırın başına # koyun ya da shell.toml [clipboard] enabled = false yazın

# Nerede: Pano bölümü listesinde bir öğe. Anlam: $tool adlı araç şu an çalışıyor (ayar dosyasında bulunmadı).
shell-doctor-clipboard-running = { $tool } (çalışıyor)

# Nerede: Görünüm bölümü. Anlam: görünüm dosyalarının bazıları başka bir kullanıcıya (ör. root) ait; $list yollar.
shell-doctor-look-not-owned = Başka kullanıcıya ait görünüm dosyaları: { $list }

# Nerede: Öneri satırı. Anlam: Nuxut başka kullanıcıya ait görünüm dosyasını yedekleyip devralabilir; komut adı çevrilmez. Sahiplik komutu (sudo/chown) önerilmez.
shell-doctor-hint-look-not-owned = Nuxut bu dosyayı devralabilir: Ayarlar › Görünüm (ya da nuxut-shell look apply); eski hâli yedeklenir

# Nerede: Bölüm başlığı. Anlam: disk alanı ve fontlar.
shell-doctor-section-system = Disk ve bağımlılıklar

# Nerede: Kurulum bölümü. Anlam: ana ikililerin hepsi kurulu; dizin ve sayı. Değişkenler: $dir, $count.
shell-doctor-binaries-ok = İkililer kurulu: { $dir } ({ $count })

# Nerede: Kurulum bölümü. Anlam: bazı ana ikililer eksik; eksiklerin adları. Değişkenler: $dir, $names.
shell-doctor-binaries-missing = { $dir } içinde eksik ikili: { $names }

# Nerede: Öneri satırı. Anlam: kurulumu yeniden çalıştırmak için komut. Değişkenler: $cmd.
shell-doctor-hint-reinstall = Kurulumu yeniden çalıştırın: { $cmd }

# Nerede: Öneri satırı (komutsuz). Anlam: kurulum betiği eksik dosyaları yeniden yazar.
shell-doctor-hint-reinstall-plain = Kurulumu yeniden çalıştırmak bunu düzeltir

# Nerede: Kurulum bölümü. Anlam: yardımcı ikililer (görsel çözücü süreci, portal, seçiciler) kurulu. Değişkenler: $count.
shell-doctor-helpers-ok = Yardımcı ikililer kurulu ({ $count })

# Nerede: Kurulum bölümü. Anlam: bazı yardımcı ikililer eksik. Değişkenler: $names.
shell-doctor-helpers-missing = Eksik yardımcı ikili: { $names }

# Nerede: Kurulum bölümü. Anlam: ikili dizini terminalin PATH'inde. Değişkenler: $dir.
shell-doctor-path-ok = { $dir } PATH'te

# Nerede: Kurulum bölümü. Anlam: ikili dizini PATH'te değil; terminalden "nuxut-shell" adıyla çalışmaz. Değişkenler: $dir.
shell-doctor-path-missing = { $dir } PATH'te değil (terminalden adıyla çalışmaz)

# Nerede: Öneri satırı. Anlam: dizin oturumun PATH'inde; bu terminal eklemeden önce açılmış. Yeni terminaller görür, açık olanlar yeniden açılmalı.
shell-doctor-hint-path-reopen-terminals = Yeni açılan terminallerde etkin; açık terminalleri yeniden açın

# Nerede: Kurulum bölümü. Anlam: ikili dizini oturum ortamlarında da (systemd kullanıcı yöneticisi, Hyprland, çalışan kabuk) PATH'te; $list bu ortamların adları. Değişkenler: $dir.
shell-doctor-session-path-ok = { $dir } oturum ortamında da PATH'te ({ $list })

# Nerede: Kurulum bölümü. Anlam: ikili dizini şu ortamların PATH'inde yok; Nuxut'un başlattığı komutlar ve kısayollar ikilileri bulamayabilir. $list ortam adları (systemd, Hyprland, nuxut-shell). Değişkenler: $dir.
shell-doctor-session-path-missing = { $dir } oturum ortamında PATH'te değil: { $list } (kısayollar ve kabuğun başlattığı komutlar bulamayabilir)

# Nerede: Kurulum bölümü. Anlam: oturum ortamlarının PATH'i okunamadı (bilgi).
shell-doctor-session-path-unknown = Oturum ortamının PATH'i okunamadı

# Nerede: Kurulum bölümü. Anlam: kabuk ikili dizinini bu oturumun PATH'ine kendisi ekledi (Hyprland ve systemd ortamı; bilgi). Değişkenler: $dir.
shell-doctor-session-path-applied = Kabuk { $dir } dizinini bu oturumun PATH'ine ekledi

# Nerede: Öneri satırı. Anlam: kabuk çalışıyor ama dizin oturumda yok; kabuk açılışta kendisi ekler, yeniden başlatmak yeter. Komut yazılmaz.
shell-doctor-hint-session-path-restart = Kabuk açılışta dizini oturumun PATH'ine kendisi ekler: kabuğu yeniden başlatın, sonra açık terminalleri yeniden açın

# Nerede: Öneri satırı. Anlam: kabuk çalışmıyor; açıldığında dizini oturumun PATH'ine kendisi ekler. Komut yazılmaz.
shell-doctor-hint-session-path-start = Kabuk açıldığında dizini oturumun PATH'ine kendisi ekler; yeni açılan terminallerde etkin olur

# Nerede: Raporda her önerinin başındaki etiket. Anlam: sorunu gideren adım.
shell-doctor-fix-label = Çözüm

# Nerede: Kurulum bölümü. Anlam: kurulu sürüm ve derlendiği kaynak commit'i. Değişkenler: $version, $commit.
shell-doctor-version = Sürüm { $version } (commit { $commit })

# Nerede: Kabuk bölümü. Anlam: kabuk süreci çalışmıyor.
shell-doctor-shell-not-running = Kabuk çalışmıyor

# Nerede: Öneri satırı. Anlam: kabuğu başlatmanın yolu.
shell-doctor-hint-start-shell = Başlatmak için: systemctl --user start nuxut-shell (ya da oturumu yeniden açın)

# Nerede: Kabuk bölümü. Anlam: kabuk var ama tanı isteğine anlaşılır yanıt vermedi (ör. eski sürüm).
shell-doctor-shell-unreadable = Kabuk çalışıyor ama durum bilgisi alınamadı (eski sürüm olabilir)

# Nerede: Öneri satırı. Anlam: kabuğu yeniden başlatmak.
shell-doctor-hint-restart-shell = Kabuğu yeniden başlatın: systemctl --user restart nuxut-shell

# Nerede: Kabuk bölümü. Anlam: kabuk çalışıyor; sürümü ve commit'i. Değişkenler: $version, $commit.
shell-doctor-shell-running = Kabuk çalışıyor: { $version } (commit { $commit })

# Nerede: Kabuk bölümü. Anlam: kabuk systemd kullanıcı birimi olarak çalışıyor (önerilen yol).
shell-doctor-service-unit = systemd kullanıcı birimiyle başlatılmış (nuxut-shell.service)

# Nerede: Kabuk bölümü. Anlam: kabuk oturumun içinden (Hyprland'in exec-once'ı) başlatılmış.
shell-doctor-service-session = Oturumun içinden başlatılmış (oturum scope'u)

# Nerede: Kabuk bölümü. Anlam: kabuk başka bir uygulamanın (ör. terminal) altında, oturum dışında başlatılmış.
shell-doctor-service-app-scope = Bir uygulamanın altında başlatılmış (oturum kapsamı dışında)

# Nerede: Kabuk bölümü. Anlam: kabuğun nasıl başlatıldığı anlaşılamadı.
shell-doctor-service-unknown = Nasıl başlatıldığı belirlenemedi

# Nerede: Öneri satırı. Anlam: kabuğu systemd birimiyle başlatmak oturum işlemlerini sağlamlaştırır.
shell-doctor-hint-service = Kabuğu birimle başlatın: systemctl --user restart nuxut-shell

# Nerede: Kabuk bölümü. Anlam: yalnızca bir kabuk süreci var.
shell-doctor-instances-one = Tek örnek çalışıyor

# Nerede: Kabuk bölümü. Anlam: aynı ikiliden birden çok süreç çalışıyor. Değişkenler: $count.
shell-doctor-instances-many = Aynı ikiliden { $count } süreç çalışıyor

# Nerede: Öneri satırı. Anlam: fazladan süreçler test ya da eski örnek olabilir.
shell-doctor-hint-instances = Fazladan süreçler test örneği ya da takılı kalmış eski bir kabuk olabilir

# Nerede: Kabuk bölümü. Anlam: kabuğun giriş oturumu doğrudan bulunuyor (kilitle/oturumu kapat çalışır).
shell-doctor-session-pid = Giriş oturumu bulunuyor

# Nerede: Kabuk bölümü. Anlam: oturum yalnızca yedek yolla bulunuyor; kilit/çıkış çalışır ama kabuk oturum dışında başlamış.
shell-doctor-session-auto = Giriş oturumu yalnızca yedek yolla bulunuyor (session/auto)

# Nerede: Kabuk bölümü. Anlam: giriş oturumu bulunamadı; kilitle/oturumu kapat çalışmaz.
shell-doctor-session-missing = Giriş oturumu bulunamadı (kilitle ve oturumu kapat çalışmaz)

# Nerede: Hyprland bölümü. Anlam: kullanıcının Hyprland ayar dosyası ve Nuxut'un ayar dosyasını dahil etme durumu. Değişkenler: $file, $state.
shell-doctor-hypr-source = Ayar dosyası { $file }: { $state ->
        [present] Nuxut ayarları dahil
        [added] Nuxut ayarları bu oturumda dahil edildi
        [off] eşitleme kapalı
        [disabled_by_user] eşitleme kullanıcı tarafından kapatılmış
        [removed_by_user] dahil etme satırı kullanıcı tarafından kaldırılmış
        [undone] eşitleme geri alınmış
        [inactive] bu örnekte eşitleme yok
        [pending] henüz denetlenmedi
        [missing_config] ayar dosyası bulunamadı
        [unsupported] ayar biçimi desteklenmiyor
       *[other] { $state }
    }

# Nerede: Öneri satırı. Anlam: Nuxut kısayol ve ayarlarını Hyprland'e yeniden eklemek.
shell-doctor-hint-hypr-source = Yeniden eklemek için: nuxut-shell msg hypr add-source

# Nerede: Hyprland bölümü. Anlam: Nuxut kısayollarıyla çözülmemiş çakışma yok (bilerek kullanıcıda kalanlar ayrı satırda).
shell-doctor-hypr-conflicts-none = Çözülmemiş kısayol çakışması yok

# Nerede: Hyprland bölümü. Anlam: kaç Nuxut kısayolu kullanıcının bind'i yüzünden devralınamıyor ve listesi (tuş, eylem, dosya:satır: neden). Değişkenler: $count, $list.
shell-doctor-hypr-conflicts = { $count } kısayol çözülemedi: { $list }

# Nerede: Öneri satırı. Anlam: kullanıcı Ayarlar'da bu kısayol için kendi tuşunu korumayı ya da Nuxut'a başka tuş atamayı seçebilir.
shell-doctor-hint-conflicts = Ayarlar › Kısayollar'da bu satırlarda "Benimkini koru"yu seçin ya da Nuxut'a başka bir tuş atayın

# Nerede: çakışma nedeni (liste öğesinde). Anlam: kullanıcının bind'i Nuxut'un ayar dosyasından sonra okunuyor, Nuxut onu kaldıramıyor.
shell-doctor-conflict-read-after = Nuxut'un satırından sonra okunuyor

# Nerede: çakışma nedeni (liste öğesinde). Anlam: Nuxut'un ayar dosyası Hyprland'e yüklenmiyor.
shell-doctor-conflict-not-loaded = Nuxut'un ayar dosyası yüklenmiyor

# Nerede: çakışma nedeni (liste öğesinde). Anlam: bind Caps Lock gibi özel bir değiştirici kullanıyor.
shell-doctor-conflict-special-mods = özel değiştirici (Caps Lock vb.)

# Nerede: çakışma nedeni (liste öğesinde). Anlam: Lua ayarında bind'in hangi dosyada olduğu bilinemiyor.
shell-doctor-conflict-unlocated = yeri bilinmiyor (Lua ayarı)

# Nerede: Hyprland bölümü, bilgi. Anlam: bu tuşlarda kullanıcının kendi Hyprland kısayolu bilerek geçerli (pencere yönetimi ya da kullanıcının seçimi). Değişkenler: $count, $list.
shell-doctor-hypr-keys-user = { $count } kısayolda Hyprland ayarınızdaki tuş geçerli (bilerek): { $list }

# Nerede: Öneri satırı. Anlam: kullanıcı isterse Ayarlar'dan tek tek Nuxut'un kısayoluna geçebilir.
shell-doctor-hint-keys-user = İsterseniz Ayarlar › Kısayollar'da "Nuxut'unkini kullan" ile tek tek Nuxut'a geçirebilirsiniz

# Nerede: Hyprland bölümü, bilgi. Anlam: bu tuşları Nuxut devraldı; kullanıcının dosyası değişmedi. Değişkenler: $count, $list.
shell-doctor-hypr-keys-taken = { $count } kısayolu Nuxut kullanıyor (dosyanız değişmedi): { $list }

# Nerede: Hyprland bölümü. Anlam: kullanıcının Hyprland ayarında hazır tema paketi izi yok. Değişkenler: $file.
shell-doctor-pack-none = Hazır tema paketi izi yok ({ $file })

# Nerede: Hyprland bölümü. Anlam: kullanıcının ayarı bir hazır tema paketini (end4 gibi) yüklüyor; paket adları ve dosya:satır. Değişkenler: $packs, $list.
shell-doctor-pack-found = Hazır tema paketi yükleniyor: { $packs } ({ $list })

# Nerede: Öneri satırı. Anlam: başka bir paketin bar'ı/kısayolları Nuxut'la çakışabilir.
shell-doctor-hint-pack = Paketin kendi bar'ı, kilit ekranı ve kısayolları Nuxut'la çakışabilir; o satırları devre dışı bırakmayı düşünün

# Nerede: Görünüm bölümü. Anlam: birleşik görünüm açık/kapalı ve ayarları kimin yazdığı. Değişkenler: $unified, $writer.
shell-doctor-look-writer = Birleşik görünüm { $unified ->
        [on] açık
       *[off] kapalı
    }; yazıcı: { $writer }

# Nerede: Görünüm bölümü. Anlam: tema ayarlarını ezen ortam değişkeni yok.
shell-doctor-look-conflicts-none = Görünümü ezen ortam ayarı yok

# Nerede: Görünüm bölümü. Anlam: tema ayarlarını ezen ortam değişkenlerinin adları ve kaynakları. Değişkenler: $list.
shell-doctor-look-conflicts = Görünümü ezen ortam ayarı: { $list }

# Nerede: Öneri satırı. Anlam: bu değişkenler başka bir araçtan ya da dosyadan geliyor.
shell-doctor-hint-look-conflicts = Bu değişkenleri koyan dosyayı (environment.d, Hyprland env satırı) düzenleyin

# Nerede: Görünüm bölümü. Anlam: Nuxut'un yazdığı dosyaları başka bir araç yönetmiyor.
shell-doctor-look-external-none = Görünüm dosyalarını başka araç yönetmiyor

# Nerede: Görünüm bölümü. Anlam: bazı görünüm dosyalarını başka bir araç yönetiyor; ayar, yol ve neden. Değişkenler: $list.
shell-doctor-look-external = Başka bir aracın yönettiği dosyalar: { $list }

# Nerede: Öneri satırı. Anlam: Nuxut bu dosyalara dokunmaz; diğer aracı kapatmak ya da dosyayı devretmek gerekir.
shell-doctor-hint-look-external = Nuxut bu dosyalara dokunmaz; diğer aracı kapatın ya da Ayarlar › Görünüm'den Nuxut'a devredin

# Nerede: Görünüm bölümü. Anlam: sürekli geri yazıldığı için durdurulan ayarlar. Değişkenler: $list.
shell-doctor-look-paused = Sürekli geri yazıldığı için durdurulan ayarlar: { $list }

# Nerede: Ses bölümü. Anlam: varsayılan çıkış ve yolun durumu. Değişkenler: $state.
shell-doctor-audio-sink = Varsayılan çıkış: { $state }

# Nerede: Ses bölümü. Anlam: varsayılan giriş ve yolun durumu. Değişkenler: $state.
shell-doctor-audio-source = Varsayılan giriş: { $state }

# Nerede: Ses bölümü, çıkış/giriş durumu. Anlam: ses seviyesi donanımın etkin bağlantı noktasından ayarlanıyor (doğru yol).
shell-doctor-audio-route = donanım bağlantı noktasından ayarlanıyor

# Nerede: Ses bölümü, çıkış/giriş durumu. Anlam: kart dışı aygıt; seviye doğrudan aygıt özelliğinden ayarlanıyor (doğru yol).
shell-doctor-audio-props = aygıt özelliğinden ayarlanıyor

# Nerede: Ses bölümü, çıkış/giriş durumu. Anlam: kartın bağlantı noktası listesi henüz okunmadı.
shell-doctor-audio-await = kartın bağlantı noktaları henüz okunmadı

# Nerede: Ses bölümü, çıkış/giriş durumu. Anlam: ses kartı yazılım seviyesinden ayarlanıyor; donanım karıştırıcısı değişmiyor.
shell-doctor-audio-card-props = yalnızca yazılım seviyesi değişiyor (donanım karıştırıcısı değil)

# Nerede: Ses bölümü, çıkış/giriş durumu. Anlam: varsayılan aygıt yok.
shell-doctor-audio-none = yok

# Nerede: Öneri satırı. Anlam: ayrıntılı ses tanısı için komut.
shell-doctor-hint-audio = Ayrıntı için: nuxut-shell msg audio-diagnose

# Nerede: Ses bölümü. Anlam: kabuğun ses servisi çalışmıyor.
shell-doctor-audio-no-service = Ses servisi çalışmıyor

# Nerede: Ses bölümü. Anlam: PipeWire'a bağlı; seviyeyi yazan yol (wpctl ya da native). Değişkenler: $backend.
shell-doctor-audio-connected = PipeWire'a bağlı (yazım: { $backend })

# Nerede: Ses bölümü. Anlam: PipeWire'a bağlanılamadı.
shell-doctor-audio-disconnected = PipeWire'a bağlı değil

# Nerede: Portal bölümü. Anlam: Nuxut portal dosyalarının hepsi kurulu. Değişkenler: $count.
shell-doctor-portal-files-ok = Portal dosyaları kurulu ({ $count })

# Nerede: Portal bölümü. Anlam: eksik portal dosyaları. Değişkenler: $list.
shell-doctor-portal-files-missing = Eksik portal dosyası: { $list }

# Nerede: Portal bölümü. Anlam: terminalin masaüstü adı Nuxut (portal seçimi doğru).
shell-doctor-portal-desktop-ok = Masaüstü adı Nuxut (XDG_CURRENT_DESKTOP)

# Nerede: Portal bölümü. Anlam: masaüstü adı Nuxut içermiyor; Nuxut portalı seçilmez.
shell-doctor-portal-desktop-other = Masaüstü adı Nuxut değil (XDG_CURRENT_DESKTOP)

# Nerede: Portal bölümü. Anlam: masaüstü adı değişkeni hiç yok.
shell-doctor-portal-desktop-unset = XDG_CURRENT_DESKTOP tanımlı değil

# Nerede: Öneri satırı. Anlam: Nuxut oturumu bu değişkeni ayarlar; eski ortamdan açılmış terminal için oturumu yeniden açmak gerekir.
shell-doctor-hint-relogin = Nuxut oturumunda bu değer ayarlanır; oturumu kapatıp açın

# Nerede: Portal bölümü. Anlam: portal seçim dosyası yok. Değişkenler: $file.
shell-doctor-portal-conf-missing = Portal seçim dosyası yok ({ $file })

# Nerede: Portal bölümü. Anlam: portal seçim dosyasını kullanıcı sahiplenmiş; Nuxut dokunmuyor. Değişkenler: $file.
shell-doctor-portal-conf-foreign = Portal seçim dosyası sizin ({ $file }); Nuxut dokunmuyor

# Nerede: Portal bölümü. Anlam: portal seçimini Nuxut yönetiyor ve dosya seçicisi Nuxut'unki. Değişkenler: $file.
shell-doctor-portal-conf-ok = Portal seçimi Nuxut'ta, dosya seçici Nuxut ({ $file })

# Nerede: Portal bölümü. Anlam: portal seçimini Nuxut yönetiyor ama dosya seçici başka (bilinçli olabilir). Değişkenler: $file.
shell-doctor-portal-conf-no-chooser = Portal seçimi Nuxut'ta, dosya seçici başka bir arka uçta ({ $file })

# Nerede: Öneri satırı (portal seçim dosyası kullanıcının). Anlam: Nuxut dosya seçicisini kullanmak için dosyada FileChooser anahtarını nuxut yapmak. Değişkenler: $file.
shell-doctor-hint-portal-foreign = Nuxut dosya seçicisi için { $file } içinde org.freedesktop.impl.portal.FileChooser=nuxut yazın, sonra oturumu kapatıp açın

# Nerede: Öneri satırı (dosya seçici başka arka uçta). Anlam: kullanıcının hyprland-portals.conf dosyasındaki FileChooser satırı Nuxut'u devre dışı bırakıyor; satır silinip seçim dosyası yeniden yazılmalı. Değişkenler: $cmd.
shell-doctor-hint-portal-chooser = Nuxut dosya seçicisi için ~/.config/xdg-desktop-portal/hyprland-portals.conf içindeki FileChooser satırını silin, sonra { $cmd } çalıştırıp oturumu kapatıp açın

# Nerede: Portal bölümü. Anlam: ekran paylaşımında Nuxut seçicisi yapılandırılmış.
shell-doctor-portal-picker-ok = Ekran paylaşım seçicisi yapılandırılmış (xdph.conf)

# Nerede: Portal bölümü. Anlam: xdph.conf var ama Nuxut seçici bloğu yok.
shell-doctor-portal-picker-missing = xdph.conf'ta Nuxut ekran paylaşım seçicisi yok

# Nerede: Portal bölümü. Anlam: xdph.conf dosyası yok.
shell-doctor-portal-picker-no-file = xdph.conf yok (ekran paylaşım seçicisi yapılandırılmamış)

# Nerede: Kilit bölümü. Anlam: kilit ekranı ikilisi kurulu. Değişkenler: $file.
shell-doctor-lock-binary-ok = Kilit ekranı kurulu ({ $file })

# Nerede: Kilit bölümü. Anlam: kilit ekranı ikilisi yok; kilitleme çalışmaz. Değişkenler: $file.
shell-doctor-lock-binary-missing = Kilit ekranı bulunamadı ({ $file })

# Nerede: Kilit bölümü. Anlam: ayarda kilit isteklerini dinleme kapatılmış.
shell-doctor-lock-listener-off = Kilit isteklerini dinleme kapalı (shell.toml [lock] listen_logind)

# Nerede: Öneri satırı. Anlam: başka kilitleyici kullanılmıyorsa ayarı açmak.
shell-doctor-hint-lock-listener = Başka bir kilitleyici kullanmıyorsanız listen_logind = true yapın

# Nerede: Kilit bölümü. Anlam: kabuk kilit isteklerini dinliyor.
shell-doctor-lock-listener-on = Kilit istekleri dinleniyor

# Nerede: Kilit bölümü. Anlam: dinleme açık ama dinleyici çalışmıyor.
shell-doctor-lock-listener-dead = Kilit dinleyicisi çalışmıyor

# Nerede: Kilit bölümü. Anlam: ayarda dinleme açık; canlı durum bu kabuk sürümünde bildirilmiyor.
shell-doctor-lock-listener-config = Kilit isteklerini dinleme ayarda açık

# Nerede: Disk bölümü. Anlam: boş alan ölçülemedi.
shell-doctor-disk-unknown = Boş disk alanı ölçülemedi

# Nerede: Disk bölümü. Anlam: ev dizininin bulunduğu diskteki boş alan (GiB). Değişkenler: $gib.
shell-doctor-disk = Boş disk alanı: { $gib } GiB

# Nerede: Öneri satırı. Anlam: derleme ve güncellemeler için yer açmak.
shell-doctor-hint-disk = Güncelleme derlemesi için en az 10 GiB boş alan bırakın

# Nerede: Disk ve bağımlılıklar bölümü. Anlam: temanın kullandığı fontların hepsi kurulu. Değişkenler: $count.
shell-doctor-fonts-ok = Tema fontları kurulu ({ $count })

# Nerede: Disk ve bağımlılıklar bölümü. Anlam: kurulu olmayan font aileleri. Değişkenler: $list.
shell-doctor-fonts-missing = Kurulu olmayan font: { $list }

# Nerede: Öneri satırı. Anlam: eksik fontların paket adları. Değişkenler: $packages.
shell-doctor-hint-fonts = Kurun: { $packages }

# Nerede: Disk ve bağımlılıklar bölümü. Anlam: Nuxut'un önerdiği paketlerin hepsi kurulu. Değişkenler: $count.
shell-doctor-packages-ok = Önerilen paketler kurulu ({ $count })

# Nerede: Disk ve bağımlılıklar bölümü. Anlam: önerilen paketlerden kurulu olmayanlar. Değişkenler: $list.
shell-doctor-packages-missing = Önerilen paket kurulu değil: { $list }

# Nerede: Disk ve bağımlılıklar bölümü. Anlam: paket veritabanı (pacman) bulunamadı; Arch dışı sistem.
shell-doctor-packages-unknown = Paket veritabanı yok (Arch dışı sistem); önerilen paketler denetlenemedi

# Nerede: Disk ve bağımlılıklar bölümü. Anlam: isteğe bağlı paketlerden kurulu olmayanlar (yalnız bilgi). Değişkenler: $list.
shell-doctor-packages-optional = İsteğe bağlı, kurulu değil: { $list }

# Nerede: Öneri satırı. Anlam: eksik paketlerin adları, paket yöneticisiyle kurulur. Değişkenler: $packages.
shell-doctor-hint-packages = Paket yöneticisiyle kurun: { $packages }

# Nerede: Kabuk bölümü. Anlam: Nuxut'la çakışan başka bir bar/bildirim programı çalışmıyor.
shell-doctor-competitors-none = Çakışan bar/bildirim programı çalışmıyor

# Nerede: Kabuk bölümü. Anlam: aynı işi yapan başka programlar çalışıyor (ikinci bar, bildirim adı çakışması). Değişkenler: $list.
shell-doctor-competitors = Çakışabilecek programlar çalışıyor: { $list }

# Nerede: Öneri satırı. Anlam: bu programları kapatıp Hyprland'deki exec-once satırlarını kaldırmak.
shell-doctor-hint-competitors = Kapatın ve Hyprland ayarındaki exec-once satırlarını kaldırın

# Nerede: Ekran okuyucu (Orca) — kabuğun ana yüzeyinin (her monitör) erişilebilir pencere adı; görünmez, yalnız sesli okunur. Anlam: masaüstü kabuğu.
shell-a11y-main = Nuxut masaüstü

# Nerede: Ekran okuyucu — bar'ın araç çubuğu adı (bar hangi kenarda olursa olsun); görünmez, sesli okunur.
shell-a11y-bar = Bar

# Nerede: Ekran okuyucu — bar'daki çalışma alanı yuvasının düğme adı; görünmez, sesli okunur.
# Değişkenler: $id (çalışma alanı numarası, örnek: 3).
shell-a11y-workspace = Çalışma alanı { $id }

# Nerede: Ekran okuyucu — bar'daki çalışma alanları göstergesinde önceki sayfa oku; görünmez, sesli okunur.
shell-a11y-workspaces-prev = Önceki çalışma alanları

# Nerede: Ekran okuyucu — bar'daki çalışma alanları göstergesinde sonraki sayfa oku; görünmez, sesli okunur.
shell-a11y-workspaces-next = Sonraki çalışma alanları

# Nerede: Ekran okuyucu — uygulama başlatıcısı yaprağının adı; görünmez, sesli okunur.
shell-a11y-launcher = Uygulama başlatıcı

# Nerede: Ekran okuyucu — başlatıcının arama alanının adı; görünmez, sesli okunur. Anlam: uygulama, pano öğesi ya da emoji aramak için yazılan alan.
shell-a11y-launcher-search = Ara

# Nerede: Ekran okuyucu — başlatıcının sonuç listesinin adı; görünmez, sesli okunur.
shell-a11y-launcher-results = Sonuçlar

# Nerede: Ekran okuyucu — başlatıcıda seçili uygulamanın eylemler sayfası (ör. yeni pencere); görünmez, sesli okunur.
shell-a11y-launcher-actions = Eylemler

# Nerede: Ekran okuyucu — açık kenar panelinin (çekmece: hızlı ayarlar, bildirimler) adı; görünmez, sesli okunur.
shell-a11y-panel = Panel

# Nerede: Ekran okuyucu (yalnız Orca okur, ekranda yok): saat popout'unun bölge adı. Anlam: tarih, saat ve takvim.
shell-a11y-clock = Takvim ve saat

# Nerede: Ekran okuyucu: açılır bildirim kartlarının grup adı. Anlam: ekranın köşesinde beliren bildirimler.
shell-a11y-notifications = Bildirimler

# Nerede: Ekran okuyucu: bildirim kartındaki kapatma (X) düğmesinin adı. Anlam: bildirimi kapat.
shell-a11y-notification-close = Bildirimi kapat

# Nerede: Ekran okuyucu: ses kartında çıkış kaydırıcısının adı. Anlam: hoparlör/kulaklık ses düzeyi.
shell-a11y-volume-output = Ses düzeyi

# Nerede: Ekran okuyucu: ses kartında çıkış cihazları listesinin adı. Anlam: sesin çalacağı cihazı seç.
shell-a11y-volume-outputs = Çıkış cihazı

# Nerede: Ekran okuyucu: ses kartında mikrofon kaydırıcısının adı. Anlam: mikrofon düzeyi.
shell-a11y-volume-input = Mikrofon düzeyi

# Nerede: Ekran okuyucu: ses kartında giriş cihazları listesinin adı. Anlam: kullanılacak mikrofonu seç.
shell-a11y-volume-inputs = Giriş cihazı

# Nerede: Ekran okuyucu: ses karıştırıcıda uygulamanın sessiz düğmesi. Anlam: $app uygulamasının sesini kapat/aç.
shell-a11y-mixer-mute = { $app } sesini kapat

# Nerede: Ekran okuyucu: gece ışığı kartındaki kaydırıcının adı. Anlam: renk sıcaklığı (Kelvin).
shell-a11y-night-light-temperature = Renk sıcaklığı

# Nerede: Ekran okuyucu: büyüteç kartındaki kaydırıcının adı. Anlam: büyütme çarpanı.
shell-a11y-magnifier-factor = Büyütme

# Nerede: Ekran okuyucu: güç kartındaki profil seçicisinin adı. Anlam: güç tasarrufu / dengeli / performans.
shell-a11y-power-profile = Güç profili

# Nerede: Ekran okuyucu: güç kartındaki şarj sınırı anahtar listesinin adı. Anlam: pili koruyan şarj sınırı.
shell-a11y-power-charge = Şarj sınırı

# Nerede: Ekran okuyucu: medya kartındaki önceki parça düğmesi. Anlam: önceki parçaya geç.
shell-a11y-media-previous = Önceki parça

# Nerede: Ekran okuyucu: medya kartındaki oynat düğmesi (duraklatılmışken). Anlam: oynatmayı başlat.
shell-a11y-media-play = Oynat

# Nerede: Ekran okuyucu: medya kartındaki duraklat düğmesi (çalarken). Anlam: oynatmayı duraklat.
shell-a11y-media-pause = Duraklat

# Nerede: Ekran okuyucu: medya kartındaki sonraki parça düğmesi. Anlam: sonraki parçaya geç.
shell-a11y-media-next = Sonraki parça

# Nerede: Ekran okuyucu: ses OSD'si değişince duyurulan metin. Anlam: ses düzeyi; $value yüzde ("%64").
shell-a11y-osd-volume = Ses { $value }

# Nerede: Ekran okuyucu: ses OSD'si, sessizdeyken. Anlam: ses kapalı.
shell-a11y-osd-volume-muted = Ses kapalı

# Nerede: Ekran okuyucu: mikrofon OSD'si. Anlam: mikrofon düzeyi; $value yüzde.
shell-a11y-osd-mic = Mikrofon { $value }

# Nerede: Ekran okuyucu: mikrofon OSD'si, sessizdeyken. Anlam: mikrofon kapalı.
shell-a11y-osd-mic-muted = Mikrofon kapalı

# Nerede: Ekran okuyucu: parlaklık OSD'si. Anlam: ekran parlaklığı; $value yüzde.
shell-a11y-osd-brightness = Parlaklık { $value }

# Nerede: Ekran okuyucu: büyüteç OSD'si. Anlam: büyütme çarpanı; $value "2,0×".
shell-a11y-osd-magnifier = Büyüteç { $value }

# Nerede: Ekran okuyucu — ekran klavyesi yüzeyinin ve tuş grubunun adı; görünmez, sesli okunur.
shell-a11y-osk = Ekran klavyesi

# Nerede: Ekran okuyucu — dosya seçicide önceki klasöre dönen ikon düğmesinin adı; görünmez, sesli okunur.
shell-a11y-filechooser-back = Geri

# Nerede: Ekran okuyucu — dosya seçicinin üst araç çubuğunun (geri, üst klasör, konum, arama) adı; görünmez, sesli okunur.
shell-a11y-filechooser-toolbar = Araç çubuğu

# Nerede: Ekran okuyucu — dosya seçicide bir üst klasöre çıkan ikon düğmesinin adı; görünmez, sesli okunur.
shell-a11y-filechooser-up = Üst klasör

# Nerede: Ekran okuyucu — dosya seçicinin yol çubuğunun (Ev › Belgeler › …) adı; görünmez, sesli okunur.
shell-a11y-filechooser-location = Konum

# Nerede: Ekran okuyucu — yol çubuğu sığmayınca soldaki "…" düğmesi; görünmez, sesli okunur. Anlam: gizlenen üst klasörler.
shell-a11y-filechooser-parents = Üst klasörler

# Nerede: Ekran okuyucu — gizli dosyaları gösteren aç/kapa ikon düğmesinin adı; görünmez, sesli okunur.
shell-a11y-filechooser-hidden = Gizli dosyaları göster

# Nerede: Ekran okuyucu — soldaki yerler, aygıtlar ve yer imleri listesinin adı; görünmez, sesli okunur.
shell-a11y-filechooser-places = Yerler ve aygıtlar

# Nerede: Ekran okuyucu — geçerli klasörün dosya listesinin adı; görünmez, sesli okunur.
shell-a11y-filechooser-files = Dosyalar

# Nerede: Ekran okuyucu — dosya türü filtresi açılır seçiminin adı; görünmez, sesli okunur.
shell-a11y-filechooser-filter = Dosya türü

# Nerede: Renk değişince yeniden açılması gereken uygulamanın adı; bildirim başlığında ve gövdesinde $app olarak (src/app/look.rs). Anlam: GNOME Dosyalar uygulaması (Nautilus).
shell-look-app-nautilus = Dosyalar

# Nerede: Renkler değişince, uygulamanın açık penceresi varken gösterilen bildirimin başlığı (src/app/look.rs). Anlam: Uygulama yeni renkleri yeniden açılınca alacak.
# Değişkenler: $app (uygulamanın adı, ör. "Dosyalar").
shell-look-app-restart-title = { $app } yeni renkleri yeniden açılınca alacak

# Nerede: Aynı bildirimin gövdesi (src/app/look.rs). Anlam: Uygulama renkleri yalnız açılışta okuyor; düğme onu yeniden başlatır, açık pencereleri kapanır.
# Değişkenler: $app (uygulamanın adı).
shell-look-app-restart-body = { $app } renkleri yalnız açılırken okuyor. Şimdi yeniden başlatırsan açık pencereleri kapanıp yeniden açılır.

# Nerede: Aynı bildirimdeki düğme (src/app/look.rs). Anlam: Uygulamayı şimdi kapatıp yeniden açar.
shell-look-app-restart-now = Şimdi yeniden başlat

# Nerede: Başka bir araç (tema yöneticisi, kurulum betiği) Nuxut'un GTK dosyalarını değiştirdiğinde ve Nuxut yeniden uyguladığında oturumda bir kez gösterilen bildirimin başlığı (src/app/look.rs). Anlam: Uygulama görünümü başka bir araçça değiştirildi, Nuxut düzeltti.
shell-look-repaired-title = Uygulama görünümü yeniden Nuxut'ta

# Nerede: Aynı bildirimin gövdesi (src/app/look.rs). Anlam: Başka bir araç GTK teması dosyalarını değiştirdi; Nuxut öncekini yedekleyip yeniden uyguladı; o aracın temasını kullanmak istenirse düğme.
shell-look-repaired-body = Başka bir araç uygulamaların temasını değiştirdi. Nuxut öncekini yedekledi ve kendi görünümünü yeniden uyguladı. O aracın temasını kullanmak istersen birleşik görünümü kapatabilirsin.

# Nerede: Aynı bildirimdeki düğme (src/app/look.rs). Anlam: Birleşik görünümü kapatıp diğer aracın temasını kullanmak.
shell-look-repaired-use-other = Bu aracı kullan

# Nerede: doctor raporu, Görünüm bölümü. Anlam: kullanıcının GTK CSS dosyaları başka bir temanın dosyalarına sembolik bağ; uygulamalar o temada kalıyor. $list yollar.
shell-doctor-look-gtk-links = Uygulama teması dosyaları başka bir temaya bağlı: { $list }

# Nerede: doctor öneri satırı. Anlam: birleşik görünüm açıkken Nuxut bu dosyaları kabuk açılışında ve dosya değişince yedekleyip kendisi devralır; Ayarlar › Görünüm'den açılır.
shell-doctor-hint-look-gtk-links = Birleşik görünüm açıkken Nuxut bunları kabuk açılışında ve dosyalar değişince yedekleyip kendisi devralır (Ayarlar › Görünüm)
