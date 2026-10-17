# Şimdi çalıyor (`now_playing`) bar widget'ı

Durum: uygulandı (2026-10-15; istek ve onay: kullanıcı).
Kod: `src/widgets/now_playing.rs`, ayarlar `src/widgets/registry.rs` (`NowPlayingSettings`),
deneme oynatıcısı `examples/fake_mpris.rs`.

## Ne gösterir

- **Yatay bar:** küçük kapak (yoksa müzik ikonu) · "Başlık · Sanatçı" (tek satır, "…") ·
  eylem ikonu: çalarken `pause` (primary), duraklatılmışken `play` (metin ve kapak soluk). İkon
  orta tıkın yapacağı eylemi gösterir — medya kartı ve GNOME/macOS/Windows medya denetimleriyle
  aynı alışkanlık (karar, 2026-10-15; ilk sürüm durum işareti gösteriyordu).
- **Dikey bar:** yalnızca kapak/ikon (hap kalınlığında kare); duraklatılmışken kapak soluk.
- **Görünürlük:** etkin oynatıcı çalıyor ya da duraklatılmışsa görünür. Durmuş oynatıcı ya da
  oynatıcı yoksa uzunluk 0 (widget ve aralığı yok).
- **İpucu:** tam başlık, sanatçılar, albüm (başlıktan farklıysa), "Oynatıcı · Durum".

## Veri

Yeni servis yok. `ShellModel::system` → `SystemModel::player()` (mevcut "etkin oynatıcı"
kuralı: en son çalmaya başlayan) ve `SystemModel::art_image()` (medya kartının kapağı).
Kapak yolu değişmedi: yerel dosya `safe_file`, ağ kapağı yalnız `[media] remote_art` açıkken,
yalnız https, `nuxut-net` SSRF-güvenli istemci, çözme ADR-0007 yalıtılmış çözücüde.

## Etkileşim (kararlar ve gerekçe)

| Girdi | Eylem | Gerekçe |
|---|---|---|
| Sol tık | `panel` hedefini aç/kapat (varsayılan sağ: medya kartı orada) | Bar düğmelerinin genel modeli (`status`, `session`, `notifications` aynı `panel` ayarı). Uç eylemi atanmışsa (`[bar.edge_actions]`) önce o gelir. |
| Orta tık | Oynat/duraklat | Yaygın alışkanlık (waybar mpris, dots-hyprland); tepside orta tık da ikincil eylem. |
| Tekerlek | `scroll = "volume"` (varsayılan): ses ± `edge_actions.scroll_step`, OSD | Durum göstergesinin ses ikonuyla aynı kural ve yol. Yanlışlıkla kaydırma geri alınabilir; parça atlamak alınamaz (önceki çoğu oynatıcıda parçayı başa sarar). |
| Tekerlek (`track`) | Yukarı sonraki, aşağı önceki; **hareket başına tek atlama** (400 ms sessizlik ya da `ScrollEnd` yeni hareket) | Dokunmatik yüzeyde tek kaydırma çok parça atlamasın. |

Orta tık ve tekerlek `overlay_pointer` ile alınır (durum göstergesinin tekerlek yolu gibi);
widget dışındaki ya da gizli widget'taki olay bar'a düşer.

## Maliyet ve hasar

- Boşta iş yok: ilerleme bar'da gösterilmez, `set_time` kullanılmaz; yalnız MPRIS olayı gelince
  `sync`. Konum (`Position`) değişimi görünümü değiştirmez → yeniden çizim yok.
- Yatay genişlik **sabit** (`width`, 120..=600, varsayılan 240): başlık ya da durum değişince
  yerleşim kaymaz; `bar_damage` yalnız bu widget'ın dikdörtgenini yeniler. Tüm şerit yalnız
  görünür↔gizli geçişinde (oynatıcı başladı/durdu) yenilenir.
- Kapak dokusu aynı `Arc` için bir kez yüklenir; widget gizlenince bırakılır.

## Varsayılan bar

Kullanıcı kararı (2026-10-15): varsayılan bar'ın ortasında saatin yanında
(`center = ["clock", "now_playing"]`). Göç yok: `bar.center`'ı elle yazmış kullanıcının listesine
dokunulmaz (yazılmamış anahtar zaten yeni varsayılanı alır); oynatıcı yokken widget gizli olduğundan
bar değişmez görünür.

## Doğrulama

Birim: `widgets::now_playing::tests` (görünürlük, metin, ipucu, ilerlemede çizim yok, sabit
genişlik, dikey ölçü, sol/orta tık, tekerlek ses/parça/kapalı, gizliyken girdi yok),
`registry::tests::now_playing_settings_parse_and_clamp`. Canlı: izole sway + `dbus-run-session`
içinde `fake_mpris` ile yatay/dikey, ipucu, orta tıkla duraklat, sol tıkla medya kartı.
