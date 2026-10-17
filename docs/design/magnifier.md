# Büyüteç (t124, erişilebilirlik)

Durum: uygulandı (2026-10-14; iş tanımı ve kısayol / varsayılan panel /
yeniden yükleme kararları 2026-10-14).

İmleç çevresinde ekranı büyütür. Mekanizma Hyprland'in kendi yakınlaştırması
(`cursor:zoom_factor`); kabuk ek yüzey/çizim yapmaz, yalnızca değeri yönetir.

## Mekanizma

- Okuma: `j/getoption cursor:zoom_factor` → `{"option":…, "float": 1.000000, "set": false}`
  (Hyprland 0.56.2'de doğrulandı). 1'in altındaki değer 1 sayılır.
- Yazma, çalışan ayar diline göre (her yazımda `j/status` `configProvider`; `hypr apply-switch`
  ile değişebilir):
  - hyprlang: `keyword cursor:zoom_factor 2.000`
  - Lua: `eval hl.config({cursor={zoom_factor=2.000}})` (`keyword` Lua altında reddedilir —
    ADR-0003 Bulgu 5).
  Değer yalnızca sayıdır (3 ondalık, 1..=10'a kırpılmış, sonlu olmayan → 1): enjeksiyon yolu yok.
- Hyprland'in aralığı: `hyprctl descriptions` → min 1, max 10 (v0.56.2). Kabuk `[magnifier] max`
  ile (varsayılan 8) daha alçak bir tavan uygular.
- Bütün Hyprland istekleri `nuxut-magnifier` thread'inde (`app/magnifier.rs`); UI beklemez.
  Kaydırıcının art arda gönderdiği yanıtsız `set`'lerden yalnızca sonuncusu uygulanır.
- **Yetki:** yazma yalnızca `SyncEnv::may_control()` doğruyken (masaüstü eşitlemesiyle aynı kapı):
  kullanıcının asıl örneği ya da `NUXUT_TEST_HYPR_CONTROL=1` + `/run/user` dışındaki
  `XDG_RUNTIME_DIR` ile yalıtılmış (sahte/iç içe) Hyprland. Test örneği kullanıcının oturumuna
  asla komut göndermez: kart gizli, `msg magnifier` hata, `status` `available:false` + `reason`.

## Adım ve sınırlar

| Anahtar | Varsayılan | Aralık | Anlam |
|---|---|---|---|
| `step` | 1.25 | 1.05..=2.0 | `in` canlı değeri çarpar, `out` böler (çarpımsal: her basış aynı oranda hissedilir) |
| `max` | 8.0 | 1.5..=10.0 | `in` ve `set` bunu aşmaz (`set` tavan üstünü **reddeder**, kırpmaz) |
| `factor` | 2.0 | 1.1..=10.0 | kapalıyken açınca (kutucuk, `on`, `toggle`); oturumda son kullanılan çarpan tercih edilir; kullanımda `max`'a kırpılır (uyarı yok: anahtarlar tek tek geçerli) |

- `out` 1.02'nin altına düşerse 1'e yapışır (göze görünmeyen kalıntı büyütme yok).
- Her komut önce **canlı** değeri okur (kullanıcının kendi bind'i ya da Hyprland ayar yeniden
  yüklemesi değeri değiştirmiş olabilir); hesap ondan yapılır.
- Geçersiz girdi (`set 0.5`, `set 10.5`, `set 2,5`, `set +2`, `NaN`, `inf`, `1e1`, fazladan
  argüman) ayrıştırıcıda reddedilir; tavan üstü `set` yürütücüde.

## Çıkışta 1'e dönüş — sahiplik kuralı

Kullanıcının elle ayarladığı büyütmeyi ezmemek için kabuk **yalnızca kendi yazdığı değeri** geri
alır (`Magnifier::managed` = kabuğun en son yazdığı 1'den büyük çarpan):

- Kapanış (SIGTERM/SIGINT/SIGHUP, `msg quit`, ve `Shell` düşerken): canlı değer `managed`'e eşitse
  (±0.001) 1'e yazılır; farklıysa dokunulmaz.
- Her okumada canlı değer `managed`'den farklıysa sahiplik bırakılır (kullanıcı değiştirmiş).
- Zaten canlı olan değere `set` (kabuk yazmadı) sahiplik yaratmaz.
- **Çökme:** kabuk büyütme yazınca `$XDG_RUNTIME_DIR/nuxut/magnifier-<HYPRLAND_INSTANCE_SIGNATURE>`
  (`0600`, atomik, tek satır çarpan) yazılır, 1'e dönünce silinir. Açılışta dosya varsa canlı değer
  ona eşitse 1'e döner, değilse dokunulmaz; dosya her durumda silinir. Dosya Hyprland örneğine
  özgüdür (büyütme Hyprland yeniden başlayınca zaten 1) ve tmpfs'tedir; bozuk içerik silinir.

## Arayüz

- **Kutucuk** `widgets/quick/magnifier.rs` (`magnifier` panel widget'ı): anahtar satırı (ikon
  `Search` — büyüteç glifi; ayrı bir `ZoomIn` ikonu framework'te yok), alt metin "Kapalı" /
  "Açık · 2,5×", açıkken 1…`max` kaydırıcısı (0,25 adım). Panel açılınca canlı değer yeniden
  okunur. Komut gönderilemiyorsa gizli. **Varsayılan panelde YOK** (2026-10-14):
  yalnızca widget ekleme menüsünden ve Ayarlar › Görünüm › Erişilebilirlik'ten eklenir.
- **IPC:** `msg magnifier status | in | out | reset | on | off | toggle | set <1-10>`.
  Eylemler `ok` ya da `error: …`; `status` tek satır JSON (sözleşme magnifier-v1, aşağıda).
  Yanıtlar tanı metnidir (ADR-0010).

## Kısayollar (2026-10-14)

| Eylem (`[keybinds]`) | Varsayılan | Tür | Komut |
|---|---|---|---|
| `magnifier_in` | `SUPER+ALT+Up` | `bindel` (kilitte de, basılı tutunca tekrar) | `msg magnifier in` |
| `magnifier_out` | `SUPER+ALT+Down` | `bindel` | `msg magnifier out` |
| `magnifier_reset` | `SUPER+ALT+Escape` | `bindl` | `msg magnifier reset` |

- **Klavye düzeninden bağımsız:** Hyprland bind'leri değiştiricisiz katmandaki keysym ile eşler;
  `=`/`+` gibi düzene göre yer değiştiren (Türkçe Q'da Shift gerektiren) tuşlar kullanılmaz. Ok
  tuşları ve Escape her düzende aynı.
- Nuxut'un öteki varsayılanlarıyla ve Hyprland'in örnek ayarının (`/usr/share/hypr/hyprland.lua`:
  `SUPER+ok tuşları` odak, `SUPER+0..9` workspace, `SUPER+mouse_down/up`) bind'leriyle çakışmaz.
  Test: `magnifier_defaults_avoid_stock_hyprland_binds` (bütün varsayılanlar birbirinden farklı +
  örnek ayar).
- Kilit ekranında da çalışır: parola alanını büyütebilmek erişilebilirlik gereği; büyütme bilgi
  sızdırmaz.
- Kullanıcının kendi bind'i aynı tuştaysa o eylem üretilmez (mevcut `split_conflicts` kuralı).

## Hyprland ayarının yeniden yüklenmesi (2026-10-14)

`reload` (ör. Lua altında `nuxut.lua` yeniden yazılınca) çalışma anında verilen değerleri ayar
dosyasındakine döndürür. Hyprland servisi socket2'deki `configreloaded`'ı ayrı bir olay
(`HyprlandEvent::ConfigReloaded`) olarak iletir; büyüteç thread'i (`Job::Reloaded`,
`services::magnifier::reapply`):
- kabuğun büyütmesi yoksa hiçbir şey göndermez;
- canlı değer kabuğunkine eşitse iş yok;
- canlı değer **1**'e döndüyse (ayar dosyası büyütme vermiyor) kabuğun çarpanını yeniden yazar;
- canlı değer **başka bir değerse** (kullanıcının ayarı büyütme veriyor) ezmez, sahipliği bırakır
  (çıkışta da dokunulmaz).

## Ayarlar sözleşmesi (Ayarlar için; istek, 2026-10-14)

**shell.toml `[magnifier]`** — üçü de `float`, şemada (`nuxut-shell schema`) `since = 2`: Ayarlar
satırları şemadan otomatik çıkar, "Yeni" rozeti `since`'ten. Başlık/açıklama şemanın `title`/
`description` alanlarında (katalogdan, kabuğun dilinde).

| Anahtar | Tür | Aralık | Adım (şema) | Varsayılan | Başlık anahtarı |
|---|---|---|---|---|---|
| `magnifier.step` | float | 1.05..=2.0 | 0.05 | 1.25 | `shell-config-magnifier-step-label` (+ `-step-describe`) |
| `magnifier.max` | float | 1.5..=10.0 | 0.5 | 8.0 | `shell-config-magnifier-max-label` |
| `magnifier.factor` | float | 1.1..=10.0 | 0.25 | 2.0 | `shell-config-magnifier-factor-label` (+ `-factor-describe`) |

- `step`: `in` canlı çarpanı bununla çarpar, `out` böler. `max`: `in`/`set` tavanı (Hyprland'in
  kendi tavanı 10). `factor`: kapalıyken açınca kullanılan çarpan (oturumda son kullanılan
  tercih edilir).
- Aralık dışı değer kırpılır + config uyarısı (`shell-config-magnifier-out-of-range`); sonlu
  olmayan değer varsayılana döner. `factor > max` uyarı DEĞİLDİR (anahtarlar tek tek geçerli);
  kullanımda `max`'a kırpılır.
- **Hızlı ayar kartı** widget kimliği `magnifier` (yalnızca panel; `since = 2`; widget ayarı yok);
  başlık `shell-bar-widget-magnifier-title`. Varsayılan panel listesinde YOK; Ayarlar eklemek
  için panelin `widgets` listesine `"magnifier"` yazar.

**Kısayollar** — `[keybinds]` eylem kimlikleri (şemada `keybinds.<eylem>`, değer `"MOD+MOD+Tuş"`
ya da `""` = atanmamış); Ayarlar etiketi `Action::title`:

| Eylem kimliği | Etiket anahtarı | Varsayılan | Komut |
|---|---|---|---|
| `magnifier_in` | `shell-keybinds-magnifier-in-label` | `SUPER+ALT+Up` | `msg magnifier in` (bindel) |
| `magnifier_out` | `shell-keybinds-magnifier-out-label` | `SUPER+ALT+Down` | `msg magnifier out` (bindel) |
| `magnifier_reset` | `shell-keybinds-magnifier-reset-label` | `SUPER+ALT+Escape` | `msg magnifier reset` (bindl) |

Varsayılanlar karardır (2026-10-14). Ayarlar varsayılanı şemadan okumalı, sabit
yazmamalı.

**Canlı durum ve eylemler (IPC, `nuxut-shell msg`):**
- `magnifier status` → tek satır JSON (sözleşme magnifier-v1; alanlar yalnızca eklenir, Ayarlar
  bilinmeyeni yok sayar):
  `{"v":1,"available":bool,"factor":number|null,"active":bool,"managed":bool,"step":number,"max":number,"reason":string|null}`
  — `available`: kabuk Hyprland'e komut gönderebiliyor ve cevap aldı; `factor`: Hyprland'in canlı
  çarpanı (`available:false` iken `null`); `active`: `factor > 1`; `managed`: canlı değer kabuğun
  yazdığı mı (kapanışta 1'e dönecek mi); `step`/`max`: geçerli ayar; `reason`: yalnızca
  `available:false` iken, tanı metni (çevrilmez, göstermek için değil).
- Eylemler: `magnifier in | out | reset | on | off | toggle | set <f>` (`f` 1..=10, ondalık nokta;
  `max` üstü reddedilir). Yanıt `ok` ya da `error: …` (tanı).
- Kabuk kapalıysa ya da komut izni yoksa (test örneği) eylemler hata, `status` `available:false`.
  Ayarlar kendi başına Hyprland'e `keyword` göndermemeli (sahiplik kuralı kabukta).

**Config izleyicisi:** Ayarlar `shell.toml`'a yazar → kabuk `config_changed` → `reload_config` →
`[magnifier]` yeni değerleri büyüteç thread'ine (`Job::Config`) gider: `step`/`max`/`factor` bir
sonraki komuttan itibaren geçerli, kartın kaydırıcı tavanı hemen güncellenir. **Canlı büyütme
değiştirilmez** — `max` canlı çarpanın altına indirilse de mevcut büyütme kalır; sonraki `in` onu
yeni tavana indirir, `set` tavan üstünü reddeder. Kısayol değişikliği masaüstü eşitlemesiyle
(`nuxut.conf`/`nuxut.lua` yeniden üretimi) uygulanır; panel `widgets` listesi değişikliği kartı
hemen ekler/kaldırır.

## Bar hapı ve OSD (2026-10-16)

İstek: kullanıcı geri bildirimi (büyüteç kısayolla açılınca "açık mı, kaç kat" görünmüyordu).

- **OSD:** çarpan değişince (kısayol, `msg`, kart ya da kullanıcının kendi bind'i — kaynaktan
  bağımsız, ses OSD'si gibi) ses/parlaklıkla aynı kapsül: arama ikonu, ray 1..=`[magnifier] max`,
  etiket yüzde yerine "2,0×" (framework `Osd::set_value_text`, eeebb00). İlk okuma ve Hyprland'e
  erişim kaybı OSD değildir (`OsdTracker::magnifier`).
- **Bar hapı:** `magnifier` artık hem panel (kart) hem bar widget'ıdır. Bar'da yalnız büyütme açıkken
  `primary` dolgulu "arama ikonu + 2,0×" hapı; kapalıyken uzunluk 0. Tık: `Command::Off` (son çarpan
  hatırlanır, `toggle` geri getirir); ipucu "Büyüteç açık: 2,0× · tıklayınca kapanır". Varsayılan
  bar'da `end`'de `privacy`'den sonra. Hyprland'e erişilemiyorsa (test örneği) hap görünmez.
- Test: `app::osd::magnifier_tests`, `widgets::magnifier_pill::tests`; canlı izole sway + sahte
  Hyprland: `msg magnifier set 2` → OSD "2,0×" ve bar hapı, `off` → ikisi de kapanır.

## Bilinen sınırlar

- `cursor:zoom_rigid`, `zoom_detached_camera` vb. yönetilmez (kullanıcının Hyprland ayarı geçerli).
- Kart açıkken başka yoldan (kullanıcının kendi bind'i) değişen değer, panel yeniden açılana kadar
  eski görünür.
- `configreloaded` sinyali dolu kanalda (64 bekleyen olay) düşebilir; o durumda büyütme yeniden
  uygulanmaz (sonraki `in`/kart açılışı doğru değeri gösterir).

## Test

- Birim: `services::magnifier` (adım/sınır, toggle hafızası, ayar doğrulama, komut metinleri,
  sahte `HyprControl` ile hyprlang/Lua yazımı, sahiplik ve çıkış, çökme işareti, yeniden
  yüklemede yeniden uygulama), `ipc` ayrıştırma, kısayol çakışmaları, kutucuk metinleri ve
  kaydırıcı eşlemesi.
- Entegrasyon: `tests/it/magnifier.rs` — gerçek `HyprlandClient` + sahte socket1: tam satırlar,
  tavan, Lua `eval`, elle değişen değere dokunmama, çökme işaretiyle geri alma, yeniden
  yüklemede geri yazma, erişilemeyen Hyprland. `tests/it/hyprland_service.rs`
  `config_reloaded_is_forwarded`: sahte socket2'deki `configreloaded>>` → `ConfigReloaded`.
- Görsel: izole sway + `examples/fake_hyprland` (büyüteç istekleri eklendi), yalıtılmış
  `XDG_RUNTIME_DIR` + `NUXUT_TEST_HYPR_CONTROL=1`: kutucuk kapalı/açık, `msg quit` sonrası
  `keyword … 1.000`, `kill -9` sonrası açılışta geri alma. Kullanıcının oturumunda canlı
  yakınlaştırma denenmedi.
