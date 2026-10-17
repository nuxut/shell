# Ses cihazı seçici (ses kartı)

Durum: uygulandı (2026-10-16). İstek: kullanıcı ("giriş/çıkış cihazı seçici");
ayrıntı kararları ürün kararı olarak burada (widgets-catalog.md, "kararlar").

## Ne

Sağ paneldeki ses kartında (`volume`), çıkış kaydırıcısının altında çıkış cihazları, giriş
kaydırıcısının altında giriş cihazları listelenir. Satıra **tek tık** o cihazı varsayılan yapar;
etkin cihaz seçili (tonal zemin) ve sağında onay işareti. Bir yönde tek cihaz varsa o yönün listesi
yoktur (seçilecek bir şey yok, kart büyümez). En çok 4 satır görünür, fazlası liste içinde kayar.

## Kararlar

1. **Ayrı widget değil, ses kartının parçası.** Kullanıcı isteği "ses kartında"; seviye ve cihaz
   aynı yerde (GNOME/KDE/Windows hızlı ayarlarıyla aynı). Karıştırıcı (`mixer`) yalnız uygulamalar.
2. **Açılır menü değil, açık liste.** "Tek tıkla değişim": menü iki tık ister. Liste yalnız 2+
   cihazda görünür, böylece tek hoparlörlü makinede kart eskisi kadar.
3. **İkon cihaz türünden** (`services::audio::DeviceClass`, servis UI sözlüğü bilmez; ikon eşlemesi
   `widgets/quick/volume.rs`'te, ikonlar framework `IconName`):

   | Tür | Nereden | İkon |
   |---|---|---|
   | Kulaklık / mikrofonlu kulaklık | Bluetooth form faktörü `headphone`/`headset`; etkin route `port.type` `headphones`/`headset`; `device.form-factor` | `Headphones` |
   | Ekran (HDMI/DP) | route `hdmi`; form faktörü `tv`/`monitor`; düğüm adında `hdmi`/`displayport` | `Monitor` |
   | Bluetooth (diğer) | `device.api = bluez5` ya da `device.bus = bluetooth` | `Bluetooth` |
   | USB | route `usb`; `device.bus = usb` | `Usb` |
   | Kamera mikrofonu | form faktörü `webcam` | `Videocam` |
   | Hoparlör / mikrofon | route `speaker`/`mic`; yoksa yöne göre | `Speaker` / `Mic` |

   Kart düğümünde etkin route'un port türü önce gelir: aynı analog çıkış, kulaklık takılınca
   kulaklık ikonuna döner (route değişimi zaten dinleniyor). İpuçları yalnız `[a-z0-9_-]`, 32 bayt.
4. **İki arka uçta aynı yol.** Varsayılan cihaz `default` metadata'sına
   `default.configured.audio.sink|source` yazılarak değişir (`AudioService::set_default`);
   `wpctl set-default` de aynısını yapar. `[audio] backend` yalnız seviye/sessiz yazımını seçer,
   cihaz seçimi iki yolda da süreç içi metadata yazımıdır. WirePlumber akışları yeni cihaza taşır
   (akışa elle hedef verilmemişse).
5. **Panel kapalıyken abonelik yok.** Yeni PipeWire aboneliği yok: cihaz listesi ve route'lar ses
   servisinin zaten tuttuğu modelden; yalnızca `NodeInfo`'ya üç sınıf ipucu eklendi. Kart, panel
   kapalıyken yalnızca model değişince eşlenir, çizim istemez.

## Testler

`services/audio/model.rs`: sınıflandırma (Bluetooth form faktörü, route port türü önceliği, HDMI
adı, webcam, USB, yön varsayılanı, geçersiz ipucu). `widgets/quick/volume.rs`: tek cihazda liste
yok, etkin işaret, etkin olmayan satıra tık → `SetDefaultAudio(id)`, etkin satıra tık → hiçbir şey,
varsayılan değişince işaret taşınır, değişmeyen durum yeniden çizim istemez. Varsayılan cihaz
yazımının kendisi `tests/it/audio_service.rs`'te (özel PipeWire) zaten sınanıyor.
