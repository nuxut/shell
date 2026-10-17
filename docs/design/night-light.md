# Gece ışığı (t113)

Akşam ekranın renk sıcaklığını düşürür (mavi azalır), sabah geri alır. Elle aç/kapa, sıcaklık
kaydırıcısı ve zamanlama (saat aralığı ya da gün batımı/doğumu). Hyprland'de çalışır; destek
yoksa özellik görünmez.

**Varsayılan kapalı** (ekip kuralı: hiçbir eylem varsayılan açık değil). Kullanıcı açınca makul
değerlerle başlar.

## Mekanizma: `hyprland-ctm-control-v1`, kabuğun içinden

Hyprland renk dönüşüm matrisini (CTM, çıkış başına 3×3) bu protokolle alır; KMS'te uygulanır,
gamma tablosuyla çakışmaz. Kabuk zaten bir Wayland istemcisi: protokolü framework'ün
`nuxut-wayland`'ı bağlar (Hyprland XML'leri `protocols/` altında gömülü üretiliyor; örnek
`hyprland-focus-grab-v1`), kabuk servisi matrisi hesaplayıp verir.

Neden `hyprsunset` süreci değil (boşluk analizindeki ilk öneri):

- Ek çalışma zamanı süreci ve paketi yok; IPC'si (`hyprctl hyprsunset …`, sürümler arasında
  değişen soket protokolü) yok.
- **Kendiliğinden güvenli:** yönetici nesnesi yok olunca (kabuk kapanır/çöker) Hyprland CTM'i
  sıfırlar — ekran turuncu kalmaz (Hyprland 0.56.2 kaynağında doğrulandı, aşağıda).
  hyprsunset'te çocuk süreç yönetimi ve ölüm sinyali gerekirdi.
- **Çakışma görünür:** protokol tek istemciye izin verir; başka istemci (kullanıcının kendi
  hyprsunset'i) varken compositor `blocked` olayı yollar (protokol sürüm 2, doğrulandı). Kabuk bunu durum olarak gösterir: "Gece ışığını başka bir uygulama yönetiyor".
  Kullanıcının aracına dokunulmaz (polkit ajanındaki ilkeyle aynı).
- Geçiş eğrisi bizde: zamanlama ve yumuşak geçiş tek yerde, saf işlevle test edilir.

Kaybedilen: hyprsunset'in kendi profil dosyası. Gerek yok — zamanlama `shell.toml`'da.

`wlr-gamma-control` (sway vb.) v1 dışı: Nuxut Hyprland kabuğu; ileride aynı servis arka ucunu
değiştirerek eklenebilir (servis arka uçtan bağımsız bir `ColorTransform` arayüzüne yazar).

## Framework yeteneği (fw e6f9d4f, 2026-10-13)

Talep karşılandı; ayrıntı framework `docs/ARCHITECTURE.md` "Renk dönüşümü". Kullanılan API
(`nuxut`'tan): `Context::color_transform_supported()` (yalnız global; bağlanmaz),
`set_color_transforms(&[(OutputId, ColorMatrix)]) -> Result` (tek atomik commit; değişmeyen
çıkış/commit gönderilmez, 1/256 `wl_fixed` yuvarlama), `color_transform_release()`,
`App::color_transform_blocked`. Açık sorular framework'ün Hyprland 0.56.2 `CTMControl.cpp`
doğrulamasıyla kapandı:
- **Kabuk ölünce:** yönetici yok olunca (istemci kapanır/çöker dahil) tüm monitörler birime
  döner; ekran turuncu kalmaz. Kapanışta framework yöneticiyi kendisi bırakır; kabuk ayrıca
  `App::terminate` (SIGTERM/SIGINT/SIGHUP) ve `msg quit`'te açıkça bırakır (istek,
  2026-10-13; `shutdown_releases_only_when_active` testi, izole sway'de sahte arka uçla SIGTERM →
  `Release` doğrulandı).
- **Kapatma = `release`,** birim matris değil: yöneticiyi tutmak kullanıcının hyprsunset'ini
  engellerdi. (Tasarımdaki "birim + release" güvencesi kaldırıldı.)
- **`blocked`:** ilk bağlanan sahip, sonrakiler `blocked` (sürüm 2, yalnız bağlanırken; Hyprland
  v2 ilan ediyor). Framework engellenmiş yöneticiyi yok eder, `set` → `Err(Blocked)` ve olay.
  Kabuk makineyi durdurur ve `release` ile framework durumunu sıfırlar; elle açma bir kez yeniden
  bağlanır. v1 compositor'da `blocked` yok → sessizce düşer (bilinen sınır).
- **Yeni çıkış:** framework kendiliğinden uygulamaz; kabuk `output_added`'da son matrisi verir.
- **Geçiş:** Hyprland `render:ctm_animation` (varsayılan auto) her değişimi 500 ms doğrusal
  söndürür; kendi ince adımlı geçişimiz her adımda bunu yeniden başlatır (zararsız).
- **Matris:** bileşenler sonlu ve ≥ 0 olmalı (Hyprland'de aksi bağlantıyı öldürür); model
  köşegeni 0..1, framework göndermeden `InvalidMatrix` döner.

## Kabuk servisi `services/night_light` (UI'dan bağımsız)

**Saf çekirdek** (`model.rs`), hepsi birim testli:

- `kelvin_to_rgb(k) -> [f32; 3]`: Tanner Helland yaklaşımı (hyprsunset ile aynı görünüm),
  çıktı 0..1, 6500 K ≈ birim. Matris köşegen: `diag(r, g, b)`.
- `Schedule`: `Always` (açıkken sürekli), `Manual { from, to }` (yerel saat, gece yarısını aşabilir),
  `Sun { lat, lon }` (gün batımı → gün doğumu). Gün batımı/doğumu `sunrise` crate'i (saf Rust,
  küçük; DEPENDENCIES'e yazılır). Kutup günü/gecesi: gün batımı yoksa o gün etkisiz / tüm gün etkin.
- `target(now, schedule, config) -> Kelvin`: geçiş bölgesinde (`transition` dakikası, sınırın
  etrafında) doğrusal karışım (mireds uzayında — algısal olarak eşit adım), dışında ya 6500 ya
  `temperature`.
- `next_wake(now, …) -> Instant`: bir sonraki değişim anı (geçişte en fazla 30 sn adım ya da
  ≥ 50 K fark; geçiş dışında bir sonraki sınır). **Boşta uyanma yok**: zamanlayıcı yalnızca bu
  ana kurulur.
- Konum: `latitude`/`longitude` ayarlı değilse saat diliminden yaklaşık konum —
  `/usr/share/zoneinfo/zone1970.tab` (tzdata, zaten kurulu) satırındaki şehir koordinatı.
  Bulunamazsa `Sun` → `Manual 20:00–07:00` ile uyarı. geoclue v1 dışı (açık nokta).

**Durum makinesi** (`mod.rs`):

- Girdi: config, saat, elle geçersiz kılma, `blocked`, uyku/uyanma (logind `PrepareForSleep(false)`
  → hemen yeniden hesapla; saat sıçramaları da aynı yoldan), yeni çıkış.
- **Elle geçersiz kılma:** hızlı ayardan aç/kapa bir sonraki zamanlama sınırına kadar geçerli
  (GNOME'daki "yarına kadar kapalı" gibi); `Always`'te kalıcı. Bellekte tutulur; yeniden
  başlatmada zamanlamaya dönülür (v1).
- Çıktı: hedef sıcaklık değiştiyse `set_color_transforms` (tüm çıkışlara aynı matris, tek
  commit). Kapalı → yalnızca `release` (Hyprland birime döndürür).
- Ayar değişince (sıcaklık kaydırıcısı) anında uygula; kaydırıcı sürüklenirken kare başına değil,
  ≤ 30 Hz birleştirme.

Ekran görüntüleri etkilenmez (CTM tarama sonrası, KMS'te uygulanır; screencopy öncesini alır) —
canlıda doğrulanacak, belgeye not.

## Ayarlar (`shell.toml [night_light]`)

| Anahtar | Varsayılan | Sınır |
|---|---|---|
| `enabled` | `false` | ana anahtar |
| `temperature` | `4000` | `1500..=6000` K (dışı kırpılır, uyarı) |
| `schedule` | `"sun"` | `"always"`, `"manual"`, `"sun"` |
| `from` / `to` | `"20:00"` / `"07:00"` | `HH:MM`; yalnızca `manual` |
| `latitude` / `longitude` | yok | yoksa saat diliminden |
| `transition` | `30` | `0..=120` dk |

Şema `SettingSchema` ile (seçenek adları `option_labels`, i18n katalogdan); Ayarlar'da
"Ekranlar" altında blok — Ayarlar'ın işi, şema hazır olunca.

## Arayüz

- Hızlı ayarlar: "Gece ışığı" anahtarı + açıkken sıcaklık kaydırıcısı (`slider_row`), alt satırda
  durum ("21:14'e kadar kapalı", "Gün doğumunda kapanır", "Başka bir uygulama yönetiyor").
  Destek yoksa satır yok.
- IPC: `nuxut-shell msg night-light on|off|toggle|status|temperature <K>` (kısayol bağlanabilir).
- Bar göstergesi v1 dışı (status widget'ına sonra eklenebilir).
- Tüm metinler `t!()`; renk kodlanmaz (tema).

## Test

- Saf: Kelvin→RGB (6500 ≈ 1, monoton), zamanlama (gece yarısı aşımı, aynı from/to, geçiş
  karışımı, DST günü, kutup günü/gecesi, saat dilimi konumu ayrıştırma), `next_wake`
  (boşta uyanma yok), geçersiz kılmanın sınırda bitmesi.
- Servis: sahte `ColorTransform` arka ucuyla (çağrı kaydı): kapalı → hiç bağlanmaz; aç → tek
  commit; blocked → durum, tekrar deneme yok; uyanmada yeniden hesap; yeni çıkışta uygulama.
- Canlı: izole sway'de CTM yok → "destek yok" yolu ve UI (kendi yüzeyini kırparak). Gerçek renk
  etkisi yalnızca Hyprland'de; kullanıcının ekranında kısa (birkaç sn, sonra birim) ve ekran
  görüntüsüyle görülemez (CTM screencopy'de yok) → kullanıcının gözüyle demo turunda doğrulanır.

## İş bölümü

1. Framework: CTM yeteneği — indi (fw e6f9d4f).
2. Kabuk: `services/night_light` saf çekirdek + durum makinesi
   (`ColorTransform` arayüzüne karşı, sahte arka uçla), `shell.toml` şeması, IPC, hızlı ayar satırı.
   Framework yeteneğine bağlandı. Kayıt satırları (services/mod.rs, config, IPC, quick) inceleme.
3. Ayarlar: Ayarlar bloğu (şema hazır olunca).

## Uygulamada netleşen kararlar

- **6500 K tam birim:** Helland eğrisi 6500 K'da tam beyaz değil (G ≈ 0,996, B ≈ 0,98); kazançlar
  6500 K değerine bölünür, nötr = birim matris (kapalıdan açığa geçişte sıçrama yok).
- **Durum metinleri** Türkçe ek uyumu (`'e/'a/'de`) gerektirmesin diye: "Kapalı · açılış 21:14",
  "Açık · kapanış 07:00", "Açık · gün doğumunda kapanır", "Kapalı · gün batımında açılır",
  "Başka bir uygulama yönetiyor". Metin her zaman bir sonraki **gerçek** değişimi söyler (gündüz
  elle açılınca, kılma gün batımında zamanlamaya devredildiği için "gün doğumunda kapanır").
- **Elle geçersiz kılma:** zamanlamanın kılmayla aynı duruma **yerleştiği** (geçişi bittiği) ilk
  sınıra kadar; zamanlama zaten o durumdaysa kılma yaratılmaz (gece "aç" → kılma yok). `always` ya
  da `enabled = false` iken oturum boyunca. Ana anahtar ya da zamanlama değişince düşer.
- **`blocked`:** kendiliğinden tekrar denenmez; kullanıcı elle açınca bir kez denenir.
- **`manual` ve `from == to`:** tüm gün etkin (gece yarısı aşımıyla aynı kural ailesi).
- **Kaydırıcı / `msg night-light temperature`** oturum belleğinde; `shell.toml`'a yazmak Ayarlar'ın
  işi (`night_light.temperature`). Config sıcaklığı değişince config geçerli olur.
- **Saat sıçraması** ek uyanma olmadan: var olan saat timer'ının her atımında duvar/monoton saat
  farkı > 2 sn ise yeniden hesap. Uyku dönüşü `PrepareForSleep(false)` (yalnızca boşta kalma
  servisi logind'e bağlıyken gelir; test örneğinde yok).
- **Saat dilimi değişimi** (2026-10-13): `services::timezone` (timedate1) bildirince
  `Shell::timezone_settled` (`app/clock.rs`, chrono önbelleği için 1,1 sn sonra) saat ve takvimle
  aynı yoldan `night_light_timezone_changed`'i çağırır: saat diliminden gelen konum yeniden
  çözülür (değiştiyse ayarlar yenilenir) ve plan yeni yerel saatle hesaplanır. `msg clock refresh`
  aynı yolu tetikler. **Bilinen sınır** (shell-reviewer, 2026-10-13): süreç ortamında `TZ`
  varsa konum ondan okunur (`location.rs`, `/etc/localtime`'dan önce); sistem dilimi değişse de
  konum o oturumda değişmez (chrono da `TZ`'ye uyar, yani saat de eski dilimde kalır — tutarlı).
- **Kapanış mandalı** (shell-reviewer önerisi, şart, 2026-10-13): `shutdown` sonrası
  `update`/`command`/`output_added` no-op; quit turu bitmeden gelen olay CTM'i yeniden bağlamaz.
- **Kart** yeni panel widget'ı `night_light` (`brightness`'tan sonra varsayılan sağ panelde).
  Panel listesini elle yazmış mevcut `shell.toml`'larda görünmez — kullanıcı ekler (göç adımı yok;
  sürüm öncesi kurulumlar config'i sıfırlıyor).
- **Şema / konum (karar: onay 2026-10-13, öneri):** `latitude`/`longitude`
  şemada yok (şema biçiminde "boş olabilir sayı" yok). v1'de Ayarlar konumu yalnızca salt okunur
  gösterir ("saat diliminden ≈ enlem/boylam"); elle konum `shell.toml [night_light]`'tan.
  Düzenlenebilir alan (boş = otomatik türü) sonraki sürüm.
- **Arka uç** (`app/night_light.rs`): `Ctm` (compositor sunuyorsa), `Unsupported` (kart gizli,
  `msg night-light` hata) ya da yalnızca test örneğinde (`NUXUT_SHELL_INSTANCE`)
  `NUXUT_NIGHT_LIGHT_FAKE=1` ile çağrıları loglayan sahte arka uç (test örneği gerçek CTM'e hiç
  dokunmaz). Makine `Context` olmadan çalışır: `set`/`commit`/`release` sıraya girer, her makine
  çağrısından hemen sonra `night_light_flush` compositor'a yollar; `Err(Blocked)` → makine durur
  ve `release`.

## Açık noktalar

- geoclue ile konum (v1: saat dilimi + elle).
- Elle geçersiz kılmanın yeniden başlatmada korunması.
- Parlaklık/gamma ile birlikte "göz konforu" (v1 yalnızca sıcaklık).
