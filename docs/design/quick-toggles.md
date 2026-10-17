# Hızlı anahtarlar kartı (`toggles`)

Durum: uygulandı (2026-10-15). Onay: kullanıcı 2026-10-15'te widget kataloğunun
(`widgets-catalog.md`) P1 adaylarının hepsini onayladı ve ayrıntı kararlarını
uygulamaya bıraktı; aşağıdaki kararlar o yetkiyle alındı.

## Ne

Kenar panelinde tek dokunuşluk kutucuk ızgarası (framework `TileGrid`/`Tile`; genişliğe göre
2–4 sütun, varsayılan sağ panelde 2): **Wi-Fi, Bluetooth, Rahatsız etme, Uyanık tut, Gece ışığı,
Koyu tema** (bu sırayla: bağlantı, dikkat, ekran — ikişerli satırlar). Her kutucuk ikon + ad +
durum alt metni; açıkken `primary` dolgu.

| Kutucuk | Durum kaynağı | Eylem | Gizli olduğu durum |
|---|---|---|---|
| Wi-Fi | NM `wifi_enabled`; alt metin ağ kartıyla aynı (SSID / Bağlı değil / Kapalı / donanım kapalı) | `SystemCommand::SetWifi` | NM yok ya da Wi-Fi cihazı yok; donanım (rfkill) kapalıysa devre dışı |
| Bluetooth | BlueZ `powered`; alt metin Bluetooth kartıyla aynı | `SystemCommand::SetBluetooth` | BlueZ ya da adaptör yok |
| Rahatsız etme | `NotificationCenter::dnd` | `NotifyCommand::SetDnd` | bildirim sunucusu bizde değil |
| Uyanık tut | `ShellModel::idle_inhibit` (elle engel) | `Action::IdleInhibit` (`msg idle inhibit` yolu) | — |
| Gece ışığı | `NightLightView::active`; alt metin kartla aynı (`status_text`) | `night_light::Command::Set` | compositor CTM sunmuyor |
| Koyu tema | `cx.theme.mode` (çözülmüş kip) | `framework.toml [colors] mode` yazılır | — |

## Kararlar

1. **Varsayılan yer: sağ panelde oturum kartının hemen altı** (`session, toggles, volume, …`).
   Gerekçe: GNOME/macOS/Windows hızlı ayarlarında olduğu gibi en sık eylemler en üstte; panel
   açılınca tek tıkla erişilir.
2. **Gece ışığı kartı varsayılan listeden çıktı** (kart silinmedi; "widget ekle" ile eklenir).
   Kutucuk aç/kapa ve durum satırını birebir verir; kartın tek fazlası oturumluk sıcaklık
   kaydırıcısı — kalıcı sıcaklık Ayarlar'da (`night_light.temperature`) ve `shell.toml`'da. Hiçbir
   ayar kaybolmuyor; panel ~1 kart kısalıyor, bildirim merkezine yer kalıyor.
3. **Ağ ve Bluetooth kartları kalıyor.** Kutucuk yalnız radyoyu açar/kapatır; ağ seçimi, şifre,
   cihaz bağlama ve eşleştirme onayı kartlarda. Bilinen çakışma: Wi-Fi/Bluetooth anahtarı hem
   kutucukta hem kartın başlık satırında. Sonraki adım (yapılmadı): kutucuğun "›" alanı
   (`TileItem::expandable`, `TileEvent::Expand`) kartın içeriğini alt sayfada (`PageStack`) açar;
   o zaman ayrı ağ/BT kartları varsayılandan çıkar (GNOME modeli).
4. **Koyu tema `framework.toml`'a yazar** (Ayarlar'ın yazdığı anahtarın aynısı, `services::color_mode`):
   tema değişimi runtime'ın dosya izleyicisinden gelir, birleşik görünüm (GTK/Qt `look.mode`)
   her zamanki yoldan izler. Kip `auto` iken anahtar sabit kipe geçer (GNOME koyu stil anahtarı
   gibi); otomatik yeniden Ayarlar'dan. Yazım `nuxut_apply::write_file` + ön koşul (dış yazım
   ezilmez, en çok 3 deneme), ayrıştırılamayan dosyaya dokunulmaz, test örneği kullanıcının
   dosyasına yazamaz (`Env::guard`). Bloklayan IO ana thread dışında (`nuxut-color-mode`, ilk
   istekte açılır; art arda isteklerden sonuncusu yazılır).
5. **İyimser kutucuk:** basınca hemen döner; her model eşlemesinde kutucuğun *gösterdiği* durum
   modelle karşılaştırılır — reddedilen istek (rfkill, izin) bir sonraki olayda geri döner.
6. **Kullanılamayan anahtar gizli**, kart hiç anahtar kalmazsa yer kaplamaz (Uyanık tut ve Koyu tema
   her zaman var). Fiyu yok (boş durum değil).
7. **Mikrofon sessiz kutucuğu eklenmedi:** ses kartının giriş satırı ve bar `[bar.edge_actions]`
   `mic` zaten var; altıncı kutucukla ızgara çift sayıda kalıyor.

## Maliyet

Yeni abonelik yok: veriler mevcut servislerin olaylarından (`ShellModel`). Kapalı panelde kart
yalnızca model değişince eşlenir, çizim istenmez. Koyu tema thread'i yalnız ilk tıklamada açılır.

## Erişilebilirlik

Kutucuklar framework `Tile` bileşeni: AT-SPI ad/rol/durum (`ToggleButton`, ad = etiket, durum =
açık/kapalı, alt metin açıklama) framework'ün erişilebilirlik ağacından gelir (atspi işi). Klavye:
panel sabitken kart henüz odak almıyor (diğer hızlı ayar kartlarıyla aynı; açık iş).

## Testler

`widgets/quick/toggles.rs`: kullanılamayanlar gizli, durum/alt metin modelden, eylem eşlemesi,
yeniden kurma yalnız küme değişince ve reddedilen iyimser dönüşün geri gelmesi.
`services/color_mode.rs`: dosya yaratma, yorum ve diğer anahtarların korunması, aynı kipte yazım
yok, bozuk dosyaya dokunulmaması, kapalı `Env`'de yazım yok.
