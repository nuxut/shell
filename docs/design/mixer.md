# Ses karıştırıcı kartı (`mixer`)

Durum: uygulandı (2026-10-16). Onay: kullanıcı 2026-10-15'te widget kataloğunun
P1 adaylarını onayladı, ayrıntı kararlarını uygulamaya bıraktı.

## Ne

Kenar panelinde, ses kartının hemen altında (varsayılan sağ panel: `session, toggles, volume,
mixer, …`): **ses çalan** her akış için iki satır — uygulama logosu + "Uygulama · içerik" (tek satır,
kesilir), altında ses kartıyla aynı satır (hoparlör düğmesi = sessiz aç/kapa, kaydırıcı
0..`max_volume`, yüzde). Akan akış yoksa kart yer kaplamaz (boş durum değil; Fiyu yok).

## Kararlar

1. **"Çalmayan uygulama listede yok":** PipeWire düğüm durumu `running` olan akışlar
   (`AudioStream::running`, yeni alan). Duraklatılmış/corked akış `idle`/`suspended`'dır ve
   görünmez; yeniden çalınca döner. Durum `node.info` STATE değişiminden okunur — ham alan
   (`pw_node_info.state`): pipewire-rs `NodeInfoRef::state()` hata metnini UTF-8 varsayıp panikler,
   kullanılmaz.
2. **Cihaz seçimi karıştırıcıda değil, ses kartında** (çıkış ve giriş; bkz. "Cihaz seçimi"):
   cihaz her zaman ses kartında, karıştırıcı yalnız uygulamalar — tekrar yok.
3. **Uygulama ikonu = logo glifi** (`app_glyph`; workspace ve paylaşım seçicisiyle aynı tablo:
   önce `application.icon-name`, sonra uygulama adı; yoksa genel uygulama ikonu). Gerekçe: dosyadan
   ikon yükleme (ikon teması + Sandbox) panel için ağır bir yol; glif tek renk, temayla uyumlu,
   IO'suz. Renkli ikon gerekirse başlatıcının yükleyicisi panel widget'larına açılabilir (sonraki adım).
4. **Akış başına satır** (uygulama başına gruplama yok): aynı uygulamanın iki akışı (iki sekme)
   içerik adıyla ayrılır; KDE/pavucontrol ile aynı. En çok 8 akış, uygulama adına göre sıralı.
5. **İki arka uç:** yazımlar ses kartıyla aynı `SetVolume`/`ToggleMute` + `AudioTarget::Node(id)`;
   wpctl yolunda `wpctl set-volume|set-mute <id>`, yerli yolda düğümün `Props`'u (akışta route yok).

## Maliyet

Yeni abonelik yok: ses servisi düğüm bilgisini zaten dinliyor; yalnız STATE biti de işleniyor
(durum değişince bir model eşlemesi). Kapalı panelde kart yalnızca model değişince eşlenir, çizim
istenmez.

## Erişilebilirlik

Bileşenler framework'ün (`Slider`, `IconButton`, `Label`); AT-SPI ağacı atspi işiyle bağlanır.
Hoparlör düğmesinin erişilebilir adı satırın uygulama adını içermeli (shell tarafı, açık iş).

## Testler

`widgets/quick/mixer.rs`: yalnız akanlar + sıralama, başlık/ikon, akan akış yokken gizli, hoparlör
düğmesine tık → `ToggleMute(Node(id))`. `services/audio/model.rs`: `set_running` ve durumun
`AudioStream`'e geçmesi. `tests/it/audio_service.rs` (özel PipeWire, wpctl + yerli): bağlanmamış
akış `running = false`; kendi sürücüsüyle akan akış `running = true`; ona seviye ve sessiz yazımı
yerli yolda. Bilinen test sınırı: WirePlumber'sız test daemon'ında `wpctl` akan düğüme yazamıyor
(gerçek oturumda wpctl WirePlumber mixer-api'siyle yazar — ses kartıyla aynı yol).
Canlı (izole sway + özel PipeWire, sahte akışlar): yalnız akan iki akış görünür, duraklatılmış gizli.
