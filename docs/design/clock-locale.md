# Saat: 12/24 saat ve saat dilimi yenileme (t119)

Tarih: 2026-10-13. Kararlar (t119: `[locale] hour_cycle`, açık biçim
kazanır, dilim değişimi yeniden başlatmadan görünür).
Uygulayan. Framework tarafı: framework 636b1b8 (`Locale::with_12_hour`,
`nuxut_i18n::time_locale` / `time_locale_for`, yalnız `hour_cycle` değişimi de
`App::locale_changed`).

## 1. 12/24 saat

`framework.toml`:

```toml
[locale]
hour_cycle = "auto"   # auto (dilin/bölgenin verisi: tr 24 s, en_US 12 s) | h23 | h12
```

**Kural:** kullanıcı `shell.toml`'da saat kalıbını **açıkça yazmışsa o kazanır**; yazmamışsa
saat `hour_cycle`'a uyar.

| Yer | Yazılmamışsa (otomatik) | Yazılmışsa |
| --- | --- | --- |
| Bar saati, yatay (`[bar.widget.clock] format`) | `Locale::format_hm`: "13:05", "1:05 PM", tr+h12 "1:05 ÖS" | strftime kalıbı |
| Bar saati, dikey (`vertical_format`) | 24 s "13\n05"; 12 s "1\n05\nPM" (gün dilimi adı locale'den; boşsa AM/PM) | strftime kalıbı |
| Saat popout'unun büyük saati | her zaman otomatik (`format_hm`) | — |
| Takvim etkinlik satırları (popout + panel kartı) | her zaman otomatik (`format_hm`) | — |
| Kilit ekranı (`lock.toml clock_format`) | otomatik (`format_hm`) | strftime kalıbı |
| Bildirim/pano zamanları | göreli ("5 dk", "dün"): saat göstermez, etkilenmez | — |

Büyük saat ve etkinlik saatleri kullanıcının bar kalıbını izlemez: bar kalıbı tarih, saniye ya
da dikey satır sonu içerebilir; bunlar tek satır saat:dakika yeridir.

### "Yazılmış mı" ayrımı

`ClockSettings { format: Option<String>, vertical_format: Option<String> }` (`lock`:
`clock_format: Option<String>`). Serde varsayılanı `None` olduğu için "yazılmamış" ile "varsayılan
değeri yazılmış" karışmaz.

- Anahtar yok **ya da `""` (yalnız boşluk dahil)** → otomatik, uyarısız. Ayarlar alanı
  boşaltınca anahtarı silebilir ya da `""` yazabilir; ikisi aynıdır.
- Geçersiz kalıp → config uyarısı + otomatik (eskiden `%H:%M`'e düşüyordu).
- **Şema:** `format` / `vertical_format` `text`, `default = ""`, `description` = "Boş bırakılırsa
  12/24 saat tercihine uyar; strftime kalıbı yazılırsa o kullanılır." (katalog anahtarı
  `shell-bar-clock-format-describe`, tr+en). Şema testi (`keys_exist_and_defaults_match_config`)
  `format = ""`'nin uyarısız ayrışıp varsayılana eşit olduğunu doğrular.

Seçenekler:

1. *Ayrı bayrak* (`format_auto = true`) — reddedildi: iki anahtar çelişebilir, Ayarlar iki
   alan yönetir.
2. *Varsayılanı `hour_cycle`'a göre değiştirmek, ama `String` tutmak* — reddedildi: "yazılmış
   `%H:%M`" ile varsayılan ayırt edilemez; kullanıcının açık seçimi h12'de ezilirdi.
3. **`Option` + boş dize = otomatik** — seçildi: tek anahtar, sıfır göç, Ayarlar'da "temizle =
   otomatik".

### Tek kaynak ve yenileme

`services::clock::time_locale()` artık framework'ün `nuxut::i18n::time_locale()`'ünü döndürür
(önceden süreç başında ortamdan `LazyLock` kopyasıydı; `[locale] formats` ve `hour_cycle`'ı
görmüyordu). Ucuz değil: bar saati widget'ı ve `CalendarView` locale'i kurulumda alıp saklar.
`App::locale_changed` (dil, `formats` ya da yalnızca `hour_cycle` değişimi) → ekranlar yeniden
kurulur (bar widget'ları yeniden yaratılır, takvim `relocalize` locale'i yeniler), saat timer
biçimi yeniden hesaplanır ve saat hemen yeniden çizilir.

### Bilinen sınırlar

- Eski kurulumdan kalma `shell.toml`'da `format = "%H:%M"` açıkça yazılıysa h12 etki etmez
  (kural gereği). Ayarlar alanı boşaltarak otomatiğe döndürür; göç yapılmaz (kullanıcının açık
  seçimi olabilir).
- tr + h12: gün dilimi adı glibc verisinden ("ÖÖ"/"ÖS"); veri boşsa (ör. de_DE) "AM"/"PM".
- Takvim ızgarasının hafta başı hâlâ `locale_from_env()` (ortam), `[locale] formats`'ı görmez
  (framework `CalendarGrid` kurulumu; ayrı iş).

## 2. Saat dilimi yenileme

Ayarlar dilimi `org.freedesktop.timedate1.SetTimezone` ile değiştirir; çalışan shell yeniden
başlatmadan yeni dilimi göstermeli.

**chrono doğrulaması (0.4.45, `src/offset/local/unix.rs`):** `Local` dilimi iş parçacığı başına
önbelleğe alır; önbellek en çok **saniyede bir** `TZ` ortam değişkenine ve `/etc/localtime`
sembolik bağının `mtime`'ına (`symlink_metadata`) bakar, değiştiyse dilimi yeniden okur.
timedated bağı yeniden yazdığından bir sonraki `Local::now()` (≥ 1 sn sonra) yeni dilimi görür.
Yani chrono kendi başına doğru, **ama** shell saati dakikada bir çizer (yeni saat ≤ 60 sn gecikir),
takvimin "bugün"ü, gün noktaları ve etkinlik saatleri (ofset kurulumda hesaplanır) bir sonraki
yeniden kuruluşa kadar eski kalır. Bu yüzden dinleyici eklendi:

- `services::timezone::TimezoneWatch` (ayrı thread `nuxut-timezone`, kendi sistem bus
  bağlantısı, UI'dan bağımsız): `org.freedesktop.timedate1` `/org/freedesktop/timedate1`
  `PropertiesChanged`'inde `Timezone` değişmiş/geçersizlenmişse `TimezoneEvent::Changed`.
  **Salt dinleme**: yöntem çağrılmaz, timedated etkinleştirilmez (`GetNameOwner` etkinleştirmez),
  kullanıcının dilimine dokunulmaz → test örneğinde (`NUXUT_SHELL_INSTANCE`) de güvenli.
  Taklit koruması: gönderici adın o anki benzersiz sahibi değilse `GetNameOwner` ile yeniden
  sorulur (timedated boşta çıkar, her etkinleşmede adı değişir); tutmazsa atılır. Bağlantı
  koparsa üstel bekleme ile yeniden bağlanılır.
- `app/clock.rs`: olay → `timezone::SETTLE` (1,1 sn; chrono önbelleği dolsun) tek atımlık timer
  (art arda gelenler birleşir) → `relocalize_screens` (dil değişimiyle aynı yol: bar, takvim,
  kartlar, gün noktaları yeni ofsetle) + takvim sorgusu yeni aralığa + saat timer'ı yeniden
  hizalanır.
- **IPC yedeği:** `nuxut-shell msg clock refresh` → aynı yol, yanıt `ok`. timedated olmayan
  sistemler ya da sinyalin kaçtığı durumlar için.

### Ayarlar sözleşmesi

1. `SetTimezone` başarıyla dönünce ek bir şey gerekmez (shell sinyali dinler, ~1,1 sn içinde
   günceller). Yedek olarak `nuxut-shell msg clock refresh` çağrılabilir (zararsız, idempotent).
2. 12/24 saat: `framework.toml [locale] hour_cycle = "auto" | "h23" | "h12"` yazılır; shell
   `locale_changed` ile anında uyar. Bar saati yalnızca `[bar.widget.clock] format`/`vertical_format`
   **yazılmamış ya da `""`** ise uyar; Ayarlar 12/24 seçimini sunarken kullanıcının açık kalıbı varsa
   bunu göstermeli ("Özel biçim kullanılıyor — temizle") ve temizlemeyi anahtarı silerek ya da `""`
   yazarak yapmalı.
3. Kilit ekranı: `lock.toml clock_format` aynı kural (yazılmamış/`""` = otomatik).

### Bilinen sınırlar

- `TZ` ortam değişkeni ayarlıysa chrono onu kullanır; timedated değişimi shell'e yansımaz (kural
  gereği: ortam kazanır).
- Saat dilimi değişimini başka yoldan yapan araçlar (`ln -sf … /etc/localtime`) sinyal üretmez:
  saat ≤ 60 sn içinde chrono yoluyla düzelir, takvim için `msg clock refresh`.
- Gece ışığı (t113, `docs/design/night-light.md`) saat diliminden konum ve zamanlama türetir:
  main'e indiğinde `Shell::timezone_settled` (`app/clock.rs`) içinden yeniden hesaplanmalı.
- Kilit ekranı (`nuxut-lock`) dilim sinyalini dinlemez: saati dakikada bir `Local::now()` ile
  çizdiği için en geç bir dakikada düzelir.
