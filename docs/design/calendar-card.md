# Saat ipucu ve Takvim kartı

Tarih: 2026-10-13. Karar veren: kullanıcı isteği (ipucu + takvimi panele kart olarak ekleyebilme)
ve kapsam kararı (aralık birleşimi); aşağıdaki kipler uygulamada belirlendi.

## 1. Saatin ipucu: gün adıyla tam tarih

Bar saatinin üzerine gelince ortak ipucu balonu (`BarWidget::tooltip`, framework `Tooltip`)
bugünün tam tarihini gösterir: `services::clock::full_date`.

| Zaman locale'i | Metin |
|---|---|
| `tr_TR` | Salı, 13 Ekim 2026 |
| `en_US` (ve bölgesiz `en`) | Tuesday, October 13, 2026 |
| `en_GB` ve diğer İngilizce bölgeler | Tuesday, 13 October 2026 |
| `de_DE` | Dienstag, 13. Oktober 2026 |
| diğerleri | `%A, %-d %B %Y` |

- Kullanıcı örneği gün adı önde ("Pazartesi, 13 Ekim 2026") istedi; Türkçede CLDR "full" kalıbı
  gün adını sona koyar ("13 Ekim 2026 Salı"), kullanıcının örneği tercih edildi.
- Sıra yerel ayara göre: ay-gün yalnızca ABD İngilizcesinde. `en_US` örneğin
  "Monday, 13 October 2026"tan farklıdır; o metin `en_GB`'de çıkar.
- Locale: saatle aynı zaman locale'i (`time_locale`: `LC_ALL` > `LC_TIME` > `LANG`, yoksa
  `en_US`), böylece ipucu bar saatiyle ve popout'un uzun tarihiyle aynı dilde olur.
- Metin her tick'te yeniden hesaplanır; gün dönünce ipucu da değişir (birim testi).
- Saat popout'u açıkken ipucu gösterilmez: popout tarihi zaten büyük gösteriyor ve balon
  popout'un üstüne düşerdi. Tıklamada bayrak hemen çevrilir; aynı olayın hover adımı açık balonu
  kapatır.
- Popout başlığındaki `long_date` ("13 Ekim 2026, Salı") değişmedi. İki biçim bilerek farklı:
  ipucu kısa ve gün adı önde, başlık tarih önde.

## 2. Takvim kartı (`calendar` widget'ı)

`WidgetKind::Calendar` ("calendar", "Takvim"/"Calendar", ikon `calendar_month`, `since = 2`)
yalnızca panele konur (`Placement { bar: false, panel: true }`). Ayarı yoktur; hafta başı ve
adlar zaman locale'inden gelir. Bar'daki takvim `clock`'un popout'udur, ayrıca bar widget'ı
olarak eklenmez. Ayarlar uygulamasının Paneller "Ekle" listesi `nuxut-shell schema` çıktısındaki
`panel = true` widget'lardan gelir; testi `schema::tests::calendar_card_is_panel_only`.

**Görünüm paylaşılır, kopya yok.** `CalendarView` iki kipte çalışır (`CalendarMode`):

| | Popout | Kart |
|---|---|---|
| Başlık (büyük saat + uzun tarih) | var | **yok** |
| İç boşluk | `space(6)` | `space(3)` (kart yüzeyi `quick::Card` çerçevesinden) |
| Etkinlik satırı | ≤5 + "+N daha" | ≤3 + "+N daha" |
| İçerik genişliği | 360 − 2·iç boşluk | panel genişliği, en çok 320 px, ortalı |

- **Büyük saat kartta gizli.** Saat bar'da, gün adlı tarih saatin ipucunda zaten var. Panelde
  ses, medya, bildirimler gibi kartlarla yer paylaşılıyor; 50 px'lik saat kartın değil popout'un
  işi. Ay/yıl ızgaranın başlığında, bugün ızgarada vurgulu.
- **Genişlik sınırı.** Hücreler kare olduğundan yükseklik genişlikle büyür. Üst/alt panelde
  (ekran boyu genişlik) sınırsız ızgara ~1500 px uzardı. Kart içeriği 320 px ile sınırlanır,
  kartta ortalanır.
- **Dar panel.** Izgara genişliğe uyar: hücre `genişlik/7`. Etkinlik başlıkları tek satırdır,
  taşan kısım "…" ile kesilir (`with_max_lines(1)`; popout'ta da uzun başlık önceden alt satıra
  taşıyordu). Saat sütunu en geniş saat metni kadardır ("Tüm gün"/"All day" sabit sütuna
  sığmıyordu). Boş liste metni sarılır.
- Ay değiştirme, gün seçme ve klavye popout'taki gibidir: oklar gün/hafta, PageUp/PageDown ay,
  Home bugün, Enter/Space seçim; Escape panele kalır. Ok tuşlarıyla taşınan seçimi liste de izler.
  Önceden ızgara 13 Kasım'ı vurgulurken liste 13 Ekim'in etkinliklerini gösteriyordu; bu düzeltme
  her iki kipte geçerli.
- Etkinlik verisi aynı `model.calendar`'dan gelir (`PanelWidget::sync`). Her görünüm kendi
  6 haftalık ızgarasının günlerini bu listeden süzer.
- Popout ve kartın metinleri katalogdan gelir (`shell-calendar-*`; dosya `i18n-allow.txt`
  geçici listesinden çıktı).

## 3. Sorgu: görünür takvimlerin birleşimi

`CalendarService` tek aralık sorgular (`set_range`). Takvim ise aynı anda birkaç yerde
görünebilir: her ekranın popout'u ve açık ya da sabit panellerdeki kartlar. Bunlar farklı aylar
gösterebilir.

- Her `Screen` görünür takvimlerinin aralıklarını verir (`Screen::calendar_ranges`): açık
  popout ve açık/sabit kenar panellerindeki `PanelWidget::calendar_range()` (kart; `quick::Card`
  iletir). Kapalı yapraktaki takvim sorguyu genişletmez.
- `calendar_view::query_range` bu aralıkları kapsayan tek aralığa birleştirir
  (`[min from, max to)`). Örnek: popout Kasım, kart Ekim → 28 Eylül..7 Aralık. Kapsayan aralık
  400 günü (`MAX_QUERY_SPAN`) aşarsa yalnızca son ay değiştiren görünümün aralığı sorgulanır;
  o görünüm artık görünmüyorsa ilk görünürünki. Servis yanıtı en çok 5000 oluşum tutar; çok
  geniş aralık yakın ayı kırpabilirdi.
- `app/calendar.rs` `CalendarQuery`: son gönderilen sorguyu ve odağı tutar. Aynı sorgu
  yinelenmez; yaprak açılınca (`force`) yine tazelenir (eski davranış). Tetikler: yaprak
  aç/kapa/sabitle (`panel_action`), ay değişimi (`Action::CalendarRange` → odak), config
  yeniden yükleme (kart eklenmiş olabilir), dil değişimi (kartlar yeniden kuruldu). Görünür
  takvim yoksa sorgu gönderilmez, son veri kalır.
- Doğrulama: birim testleri `calendar_view::tests::query_range_*` ve
  `app::calendar::tests::*`. Canlıda (izole sway + `examples/fake_calendar.rs`) sabit kartlar
  Ekim'deyken popout Kasım'a geçince tek sorgu `2026-09-28 .. 2026-12-07` geldi; popout
  kapanınca sorgu `2026-09-28 .. 2026-11-09`a döndü.

`examples/fake_calendar.rs`: görsel test için sahte `org.nuxut.Calendar1`. Yalnızca
`dbus-run-session` özel bus'ında çalıştırılır; her sorguyu stderr'e yazar.

## Bilinen sınırlar

- **Dar kartta ızgara sıkışık (framework talebi).** Yaklaşık 280 px'ten dar panelde hücre
  ~32 px'in altına iner. `CalendarGrid`'in kısa gün adları üst üste biner ("PrşCumCts"), etkinlik
  noktaları rakamlara değer ve uzun ay başlığı ("October 2026") önceki-ay düğmesinin üstüne
  taşar. Bunlar framework bileşeninin içinde. İstenen: hücre dar olunca dar gün adları
  (CLDR narrow, ör. "P S Ç P C C P") ve nokta yerleşimi, başlığın düğmelerle kırpılması.
  Shell'de geçici kopya yazılmadı. Varsayılan panel genişlikleri (340/380) etkilenmez.
- Çok uzak aylardaki görünümlerde (birleşim > 400 gün) yalnızca odaktaki görünümün etkinlikleri
  gelir; diğerinin noktaları o sırada boş kalır.
- Dil değişiminde kart yeniden kurulur ve bugünün ayına döner (diğer panel widget'ları gibi).
- İpucu metni, balon açıkken gece yarısı değişirse bir sonraki işaretçi hareketinde yenilenir.
