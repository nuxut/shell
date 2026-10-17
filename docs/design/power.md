# Güç: profiller ve pil şarj sınırı (t117)

Taslak ve kabuk servisi 2026-10-13. Ayarlar sayfası
Ayarlar'ın; bu not onun planını da içerir. Kararlar karar veren + tarihle işaretlenir;
kullanıcı kararı gereken UX seçimleri **Açık sorular**'dadır, tahminle karara bağlanmadı.

## Bugün

- `src/services/power/` (UPower + power-profiles-daemon, sistem bus, polling yok):
  `DisplayDevice` (birleşik pil) ile yüzde, şarj durumu, kalan süre, AC, düşük pil olayı;
  ppd varsa profiller ve `set_profile`. Hızlı panelin güç kartı bunları gösterir.
- **Şarj sınırı yoktu.** Ayarlar'da güç sayfası, UPower/ppd kodu yok.
- Bu makinede upower 1.91.5 kurulu, power-profiles-daemon kurulu değil.

## UPower'ın şarj sınırı (doğrulanmış)

Kaynak: kurulu `/usr/share/dbus-1/interfaces/org.freedesktop.UPower.Device.xml`,
`/usr/share/polkit-1/actions/org.freedesktop.upower.policy`,
`/usr/lib/udev/{rules.d,hwdb.d}/60-upower-battery.*` ve `/usr/lib/upowerd` dizgeleri (upower
1.91.5). Gerçek servise hiçbir çağrı yapılmadı.

- **Arayüz** `org.freedesktop.UPower.Device` (UPower ≥ 1.90), gerçek pil nesnesinde
  (`/org/freedesktop/UPower/devices/battery_BAT0`), birleşik `DisplayDevice`'ta değil:
  - `ChargeThresholdSupported` (b), `ChargeThresholdEnabled` (b),
    `ChargeStartThreshold` (u), `ChargeEndThreshold` (u) — salt okunur özellikler;
    eşik `G_MAXUINT` = ayarlanmadı (UPower o eşiği yazmaz).
  - `ChargeThresholdSettingsSupported` (u, 1.91): bit 1 başlangıç, bit 2 bitiş, bit 4 aygıt
    yazılımının yönettiği "optimize şarj". Yoksa eşikler olduğu gibi alınır.
  - `EnableChargeThreshold(b)`: tek yazma yolu. `true` → eşikler sysfs'e
    (`charge_control_{start,end}_threshold`) yazılır; `false` → pil her zaman %100'e dolar.
- **Eşik değerleri D-Bus'tan değiştirilemez.** udev kuralı, pil sysfs'te eşik dosyası varsa
  hwdb'den `CHARGE_LIMIT` alır; upower'ın varsayılan hwdb'si **her pil için**
  `CHARGE_LIMIT=75,80` (75'in altına düşünce şarja başla, 80'de dur). Değiştirmek kök yetkisiyle
  `/etc/udev/hwdb.d/61-battery-local.hwdb` + `systemd-hwdb update` + `udevadm trigger` ister
  (`_` bir eşiği atlar: `CHARGE_LIMIT=_,80`).
- **polkit:** eylem `org.freedesktop.UPower.enable-charging-limit`; varsayılan
  `allow_active=yes`, `allow_inactive=no` (`allow_any` tanımsız → hayır). Yani etkin yerel
  oturumda parola sorulmaz; uzak/etkin olmayan oturumda reddedilir. upowerd denetimi
  **etkileşimsiz** yapar (`is_challenge` → yetkisiz): yöneticinin kuralı `auth_admin` ise de
  diyalog açılmaz, istek reddedilir.
- **Hatalar:** polkit reddi `org.freedesktop.UPower.GeneralError` + "Operation is not allowed.";
  desteklenmiyorsa `org.freedesktop.UPower.NotSupported`; sysfs yazma hatası da `GeneralError`
  (farklı ileti). Reddi ayırmak için iletiye bakmak zorunlu (bilinen sınır, aşağıda).
- **Kalıcılık:** upowerd durumu `/var/lib/upower/charging-threshold-status`'a yazar ve
  açılışta geri yükler. Kabuğun hiçbir şey saklaması gerekmez; Nuxut kapalıyken de geçerlidir.
- **Cihaz izleme:** `org.freedesktop.UPower.EnumerateDevices()` (DisplayDevice'ı içermez),
  `DeviceAdded(o)` / `DeviceRemoved(o)`. Sistem pili = `Type` 2 ve `PowerSupply` true (fare,
  kulaklık pilleri `PowerSupply` false).

## Kabuk servisi (yapıldı, 2026-10-13)

- `PowerState::charge_limit() -> ChargeLimit`: `Unsupported` ya da
  `Supported { enabled, mixed, start: Option<u8>, end: Option<u8> }`.
- `PowerService::set_charge_limit(on) -> Option<u64>`: sonuç
  `PowerEvent::ChargeLimitDone { id, result: Result<(), ChargeLimitError> }`.
  `ChargeLimitError::{Unavailable, Unsupported, Denied, Failed(<D-Bus hata adı>)}`; içerik
  yalnızca hata adıdır (UPower iletisi log'a/yanıta taşınmaz).
- Pil izleme: `EnumerateDevices` (en çok 32 cihaz okunur, en çok 8 pil izlenir) +
  `DeviceAdded`/`DeviceRemoved`; pil `PropertiesChanged`'i yalnızca UPower'ın benzersiz
  sahibinden ve o pilin son tam okumasından yeniyse uygulanır (pil başına seri sınırı; paylaşılan
  sınır yükseltilmez ki kuyruktaki başka nesnelerin sinyalleri atılmasın). UPower düşüp geri
  gelince piller yeniden listelenir.
- İstekler **sırayla**: sonraki istek öncekinin sonucundan (ve yeniden okunan durumdan) sonra
  değerlendirilir; zaten istenen durumdaki pile çağrı gitmez. Başarıda çağrılan piller yeniden
  okunur ve yeni durum `ChargeLimitDone`'dan **önce** `Changed` ile yayınlanır (UPower'ın
  `PropertiesChanged`'i yöntem yanıtından sonra gelebilir). Bus bağlantısı koparsa bekleyen
  istekler, bağlı değilken gelenler de hemen `Unavailable` ile yanıtlanır. Hatada da çağrılan
  piller yeniden okunur (kısmi değişiklik `mixed` görünür).
- `msg` (sözleşme **power-v1**; `src/ipc.rs`, `src/app/power.rs`):

  ```text
  nuxut-shell msg power status
  {"version":1,"upower":true,"on_battery":false,
   "battery":{"percentage":81.0,"state":"charging","time_to_empty":null,"time_to_full":1800,"warning":"none"} | null,
   "profiles":{"active":"balanced","available":["power-saver","balanced","performance"],"degraded":null} | null,
   "charge_limit":{"supported":true,"enabled":true,"mixed":false,"start":75,"end":80} | {"supported":false}}
  nuxut-shell msg power charge-limit status     # yalnız charge_limit nesnesi
  nuxut-shell msg power charge-limit on|off     # ok | error: <unavailable|unsupported|denied|failed>: <açıklama>
  ```

  `on|off` yanıtı UPower cevap verince gelir (yanıt penceresi `ipc::REPLY_TIMEOUT`). Aynı anda en
  çok 4 yanıt bekler; fazlası en eskiyi yanıtsız düşürür. `toggle` yok: karışık durumda anlamı
  belirsiz, istemci durumu bilir.
- **i18n:** kabuktaki kullanıcı metni yalnızca hızlı panel kartında (aşağıda). `msg` yanıtları ADR-0010 gereği çevrilmez
  (`i18n-allow.txt`: `src/app/power.rs`); Ayarlar `error:` kodunu kendi kataloğuyla çevirir.

## Kararlar (2026-10-13)

1. **Birden fazla pil: tek anahtar, destekleyen tüm pillere uygulanır.** Gerekçe: UPower eşiği
   pil başına tutar ama kullanıcının niyeti makine başınadır ("pilimi koru"); iki anahtar
   ThinkPad'in iç + değiştirilebilir pilinde kafa karıştırır. Durum: hepsi açıksa açık;
   bazıları açıksa `mixed` (anahtar kapalı görünür, açmak hepsini açar). Desteklemeyen pil
   (ör. sonradan takılan) yok sayılır: özelliği destekleyen pilde gizlememek için. Eşikler
   yalnızca tüm destekleyen piller aynı değeri bildiriyorsa gösterilir; farklıysa `null`.
2. **Eşikler salt okunur gösterilir.** Değiştirme yolu kök yetkili hwdb dosyası; Ayarlar'dan
   yazmak polkit'li ayrı bir yardımcı ve udev tetiklemesi ister. Kapsam dışı (Açık soru 3).
3. **Varsayılan değiştirilmez.** Nuxut şarj sınırını kendiliğinden açmaz/kapatmaz; UPower'ın
   kalıcı durumu neyse odur (ekip kuralı: hiçbir eylem varsayılan açık değil).
4. **polkit reddi ayrı tür** (`Denied`); iptal de aynı türe düşer (upowerd etkileşimsiz denetler,
   ayrı "iptal" yanıtı yoktur). Log'a yalnızca tür + D-Bus hata adı yazılır.
5. ~~**Kabukta UI yok.**~~ (2026-10-15 kararla değişti: hızlı panel güç kartında anahtar var, aşağıda.) Asıl yeri yine Ayarlar.

## Hızlı panel güç kartı ve servis düzeltmeleri (2026-10-15)

**Kararlar (2026-10-15; Açık soru 1 ve 4'ü kapatır, Karar 5'in yerine geçer):**

- Şarj sınırı anahtarı hızlı panelin güç kartında da var (`src/widgets/quick/power.rs`). Yalnızca
  `charge_limit.supported` olan donanımda görünür; masaüstü/VM'de satır hiç çizilmez.
- Etiket "Pil ömrünü koru"; alt metin `end` biliniyorsa "Şarj %N olunca durur" (`NUMBER(percent)`,
  ek almaz), bilinmiyorsa "Şarj tam dolmadan durur", `mixed` ise "Pillerin bir kısmında açık".
- v1'de eşik düzenleme yok (Açık soru 3 açık kalır).
- Kartın istekleri `SystemModel::charge_requests`'te tutulur (en çok 8); yalnız onların hatası
  kartta anlaşılır metinle görünür (`shell-power-error-charge-*`). `msg` isteklerinin yanıtı yine
  `app/power.rs`'te, çevrilmeden. EN değerleri taslak.

**Düzeltmeler (inceleme şartları):**

1. *Bus yokken istek asılı kalıyordu.* Servis sistem bus'ına bağlı değilken (geri çekilme
   uykusunda) gelen komutlar atılıyordu; `msg power charge-limit on` yanıt penceresi dolana dek
   bekliyordu. `nuxut_dbus::ServiceContext::sleep_with(d, on_command)`: uykuda gelen komut
   çağırana verilir; güç döngüsü `SetChargeLimit`'i hemen `ChargeLimitDone(Err(Unavailable))`,
   `SetProfile`'ı `ProfileFailed` ile yanıtlar. Test: `requests_answered_while_bus_unavailable`
   (geçersiz bus adresi, 60 s geri çekilme, yanıt < 500 ms).
2. *Çok pilde kısmi hata görünmüyordu.* İstek pilleri sırayla çağırır; BAT0 değişip BAT1
   reddedilirse eskiden yalnız başarıda yeniden okunuyordu, model eski "kapalı" durumda
   kalıyordu. Artık sonuç ne olursa olsun çağrılan piller yeniden okunur ve durum
   `ChargeLimitDone`'dan önce yayınlanır: yarım değişiklik `mixed = true` olarak görünür. Test:
   `charge_limit_partial_failure_shows_mixed` (BAT0 kabul, BAT1 ret → `Denied`, öncesinde
   `mixed`).

## Ayarlar sayfası planı

Yer: **Sistem → Güç** (yeni sayfa). Pil yoksa (masaüstü/VM) sayfa yalnızca profilleri gösterir;
ikisi de yoksa sayfa kenar çubuğunda görünmez.

**Güç profili**

- Üç seçenekli parçalı düğme / radyo: Güç tasarrufu, Dengeli, Performans (`available`'da
  olmayan devre dışı). `degraded` doluysa Performans altında açıklama (`lap-detected`: "dizüstü
  kucakta algılandı", `high-operating-temperature`: "sıcaklık yüksek").
- **ppd yoksa** (`profiles: null`): seçim devre dışı + alt metin "Güç profilleri için
  power-profiles-daemon gerekli" (paket adı: `power-profiles-daemon`; tuned-ppd de aynı D-Bus
  adını sunar). Gizlemek yerine devre dışı: kullanıcı neden yok olduğunu öğrensin.

**Pil şarj sınırı** (yalnızca `charge_limit.supported`)

- Anahtar: "Pil ömrünü koru". Alt metin salt okunur eşiklerden: `end` varsa "Şarj %{end}'de
  durur", `start` da varsa "%{start}'in altına düşünce yeniden başlar"; ikisi `null`'sa "Şarj
  sınırını aygıt yazılımı belirler". `mixed` ise "Pillerin bir kısmında açık".
- Hata: `denied` → "İzin verilmedi (yalnızca etkin yerel oturumda değiştirilebilir)";
  `unsupported` → anahtarı gizle + durumu yenile; `unavailable` → "UPower çalışmıyor";
  `failed` → genel hata. Hata sonrası anahtar gerçek duruma döner (iyimser değil).
- Metinler taslak; Ayarlar kataloğuna tr + en.

### Ayarlar UPower'a nasıl konuşur — iki seçenek

| | A. Kabuk `msg` (power-v1, yapıldı) | B. Paylaşılan crate (`crates/nuxut-power`) |
|---|---|---|
| Emsal | Gece ışığı, Ekranlar, Varsayılan uygulamalar | Ağ/Bluetooth (`docs/design/network-settings.md`,) |
| Kabuk kapalıyken | Çalışmaz | Çalışır (UPower/ppd sistem servisi; ayar UPower'da kalıcı) |
| Canlı güncelleme | Yok: sayfa açılınca + her komuttan sonra `power status`; görünürken yoklama gerekir | `PropertiesChanged` (Ayarlar'ın `notify` kancası) |
| polkit öznesi | Kabuk süreci | Ayarlar süreci (ikisi de aynı etkin oturum; fark yok) |
| İş | Hazır | `nuxut-dbus` main'e inince taşıma: `power/{model,mod}.rs` → crate; `ProfileFailed(String)` içindeki `t!` → tür (`ProfileError`), metin kabukta (`nuxut-nm` kalıbı); sahte UPower/ppd `tests/`'ten crate'e |
| Sözleşme bakımı | JSON şeması iki repoda | Rust türleri tek yerde |

**Öneri: B** (onay bekliyor). Ağ planıyla tutarlı; güç bir sistem
ayarı ve UPower'da kalıcı, kabuğun çalışmasına bağlı olmamalı; canlı durum yoklamasız gelir.
`msg power …` yine kalır (betikler, tuş bağları, tanı). Ayarlar sayfası crate'ten önce
başlayacaksa A ile başlayabilir: JSON alanları crate türlerinin birebir karşılığıdır, geçiş
yalnızca veri kaynağını değiştirir.

**Karar: B** (2026-10-13, Ayarlar `docs/design/power-page.md`;
uygulandı 2026-10-15). `crates/nuxut-power`: `src/services/power/{model,mod}.rs`
taşındı (kabukta `services/power.rs` yalnızca yeniden dışa aktarma). API değişiklikleri:
`ProfileFailed(String)` → `ProfileDone { id, result: Result<(), PowerError> }` ve
`set_profile -> Option<u64>` (başarıda profiller yeniden okunur, yeni durum sonuçtan önce);
`ChargeLimitError` → ortak `PowerError` (ppd: `AccessDenied` → `Denied`, `InvalidArgs` →
`Unsupported`); `PowerOptions::bus` + `bus_from_env()` (`NUXUT_POWER_BUS`, `test-support`);
sahte veri için `apply_manager` / `apply_device` / `apply_profiles` / `track_battery` açık.
Profil hatası metni kabukta `system.rs::profile_error_text`.

## Açık sorular (kullanıcı kararı)

1. ~~Şarj sınırı anahtarı hızlı panelin güç kartında da olsun mu?~~ Karar: evet (2026-10-15).
2. **Hoş geldin / ilk kurulum** ekranında şarj sınırı önerilsin mi (destekleyen dizüstünde
   "pil ömrünü korumak için açın" adımı)? Varsayılan yine kapalı kalır.
3. Kullanıcı **eşik değerlerini** (ör. 60/80) Ayarlar'dan değiştirebilsin mi? Kök yetkili
   yardımcı (polkit eylemi + hwdb dosyası + udev tetikleme) gerektirir; ayrı güvenlik
   incelemesi ister.
4. ~~Anahtar adı ve alt metin~~ Karar: "Pil ömrünü koru" + "Şarj %N olunca durur" (2026-10-15); yüzde cihazın eşiğinden gelir.

## Bilinen sınırlar

- polkit reddi, `GeneralError` iletisindeki "not allowed" ile ayırt edilir (upowerd ayrı hata
  adı vermiyor); UPower iletiyi değiştirirse ret `Failed(GeneralError)` olarak görünür (işlev
  bozulmaz, yalnızca metin genelleşir).
- `PropertiesChanged`'deki `invalidated` listesi kullanılmaz (UPower değer gönderir).
- Ayarlar A yolunda canlı değil: başka bir araç (`upower`, başka masaüstü) durumu değiştirirse
  sayfa yeniden okuyana kadar eski görünür.
