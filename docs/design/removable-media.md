# Çıkarılabilir medya (v1)

Görev t122; kararlar 2026-10-13.
Boşluk analizi satırı: `docs/design/desktop-services.md` #15.

## Kapsam

USB bellek / hafıza kartı takılınca "Aç / Çıkar" düğmeli bildirim; düğmeler ve
`nuxut-shell msg removable …` ile bağla+aç ve ayır+güç kes. v1 dışı: hızlı panel listesi,
kendiliğinden bağlama, kilitli kripto kabı için kilit açma istemi, optik disk özel davranışı.

Kod: `src/services/removable_media/` (UI'dan bağımsız servis; `model.rs` saf süzgeç ve etiket),
`src/app/removable_media.rs` (bildirimler, `msg`), sahte udisks `tests/support/udisks.rs`
(entegrasyon testleri ve `examples/fake_udisks.rs` canlı test).

## Kaynak: udisks2

`org.freedesktop.UDisks2` (sistem bus'ı) ObjectManager'ı (`/org/freedesktop/UDisks2`) bir kez
`GetManagedObjects`, sonra `InterfacesAdded`/`InterfacesRemoved` ve `PropertiesChanged` ile
izlenir; yalnızca `Block`, `Filesystem`, `Drive` arayüzleri tutulur (sınırlı sayıda). Sinyaller
adın o anki benzersiz sahibinden ve son tam okumadan yeniyse uygulanır (taklit koruması, bluetooth
servisiyle aynı `NameOwner`). Bağlantı koparsa geri çekilmeli yeniden bağlanır.

**Birim** = `Filesystem` arayüzü olan `Block` ve: `HintIgnore` değil; `HintSystem` değil; sürücüsü
`Removable` ya da `MediaRemovable` ya da `ConnectionBus` `usb`/`sdio`. Durumlar:

| Durum | Davranış |
|---|---|
| Bölüm tablosu, uzatılmış bölüm kabı | `Filesystem` yok → birim değil; her dosya sistemi bölümü ayrı birim (ayrı bildirim). |
| Bölümlerin parça parça gelmesi | udev önce tabloyu, sonra bölümleri, yoklamadan sonra `Filesystem`'i ayrı sinyallerle getirir. Birim listesi her değişimde yeniden hesaplanır; bir birim koşulları *sağladığı anda* "eklendi" sayılır, bir kez. |
| Kilitli LUKS kabı | `Filesystem` yok → birim değil (kilit açma istemi v1 dışı). |
| Kilidi açılmış kripto (açık metin aygıtı) | `CryptoBackingDevice` dolu: kendi `Drive`'ı boşsa sürücü ve `HintSystem` arka aygıttan (dm aygıtında güvenilmez). Çıkarırken ayrılır, sonra arka aygıt `Encrypted.Lock`. |
| Loop (disk görüntüsü) | Sürücü yok (`Drive = /`) → birim değil. |
| Sistem diski, dahili SATA/NVMe | `HintSystem` ya da çıkarılabilir değil → birim değil. |

**Kimlik** = blok nesnesinin son bileşeni (`/org/freedesktop/UDisks2/block_devices/sdb1` →
`sdb1`): kısa, okunur, `[A-Za-z0-9_]{1,64}` (udisks yol bileşenlerini bu kümeye kaçışlar), komut
satırına ve yola güvenle girer; yol kimlikten yeniden kurulur. Aynı ad yeniden kullanılırsa
(bellek çıkarılıp başka bellek aynı `sdb1` ile takılırsa) eski bildirim zaten sökülmede kapanmıştır.

**Etiket**: `IdLabel`, yoksa sürücünün `Vendor Model`'i, yoksa boyut ("16 GB birim"; ondalık SI,
satıcıların yazdığı gibi).

## Kararlar

1. **Açılışta takılı olanlara bildirim yok.** Her tam okuma (açılış, udisks'in yeniden
   başlaması/etkinleşmesi) taban çizgisidir; yalnızca ondan sonra listeye giren birim bildirilir.
   Gerekçe: oturum açılışında ya da udisks yeniden başlayınca takılı her aygıt için bildirim
   yağmuru gürültüdür; kullanıcı onları zaten biliyor. udisksd adı coldplug bittikten sonra alır
   (nesneler ad sahiplenilmeden dışa açılır); bu yüzden etkinleşme anındaki okuma tamdır. Sahte
   udisks de aynı sırayla davranır (`FakeUdisks::unnamed` + `claim`).
2. **Kendiliğinden bağlama (automount) v1'de yok.** "Aç" bağlar. Gerekçe: bağlamak polkit
   istemi doğurabilir (takınca beklenmedik parola penceresi), kötü niyetli/bozuk dosya sistemini
   kullanıcı istemeden çekirdeğe bağlatır ve çıkarmayı "güvenle çıkar" zorunluluğuna çevirir.
   Sonra `[removable_media] automount` olarak eklenebilir (açık nokta).
3. **Sökülme.** Birim listeden çıkınca (söküldü, kilitlendi, udisks gitti) bekleyen "takıldı"
   bildirimi popup'tan ve merkezden kaldırılır (düğmeleri anlamsız). Kaldırılmamış eski bir
   düğmeye basılırsa (ör. merkez geçmişi) `msg` "bilinmeyen birim" hatası döner, yan etki yok.
   `msg` ile çıkarılınca da "takıldı" bildirimi kaldırılır.
4. **Etkinleştirme.** udisksd D-Bus ile etkinleşir (statik `udisks2.service`); çalışmıyorsa
   çıkarılabilir aygıtlar hiç nesne olarak görünmez. Bu yüzden masaüstü örneği
   `GetManagedObjects` çağrısıyla onu başlatır (salt okunur çağrı; dosya yöneticileri de aynısını
   yapar). **Test örneği** (`NUXUT_SHELL_INSTANCE`) gerçek sistem bus'ındaysa etkinleştirmez ve
   bağla/ayır/güç kes çağırmaz (yalnız izler; eylemler `Disabled` hatası); `DBUS_SYSTEM_BUS_ADDRESS`
   açıkça sahte bir bus'a yönlendirilmişse tam çalışır (idle/logind ile aynı kural).
5. **Çıkar = aynı sürücünün tüm dosya sistemleri.** Çok bölümlü bellekte yalnızca bir bölümü
   ayırmak güvenli çıkarma değildir: sürücüdeki (kilidi açık kriptoda arka aygıtın sürücüsü)
   bağlı **her** `Filesystem` bloğu — birim süzgecine takılanlar dahil (`HintIgnore`'lu kurtarma
   bölümü vb.; liste değil `ObjectStore` taranır) — `Unmount({})` (biri başarısızsa dur, hata
   bildirimi; `DeviceBusy` → "dosyalar bir uygulamada açık"), açık metin aygıtlarının arka aygıtı
   `Lock({})`, sonra `CanPowerOff` ise `PowerOff({})`, değilse `Ejectable` ise `Eject({})`.
   Güç kesme/çıkarma `DeviceBusy` dönerse sonuç `Failed(Busy)` ("çıkarılamadı"), "Güvenle
   çıkarılabilir" denmez. Kilit hatası ve diğer güç kesme hataları (ör. yetki) yalnızca loglanır:
   birimler ayrıldı, veri güvende.
   *Düzeltme 2026-10-13, inceleme:* ilk sürüm yalnızca listedeki kardeş birimleri
   ayırıyordu; süzgece takılan bağlı bölüm kalınca `PowerOff` reddediliyor, hata loglanıp yine
   "güvenle çıkarılabilir" deniyordu.
5a. **`Unmount` zaman aşımsız** (2026-10-13, inceleme). Yavaş USB bellekte
   önbellek boşaltma dakikalar sürebilir; çağrıyı yarıda bırakıp "zaman aşımı" demek kullanıcıyı
   belleği çekmeye iter ve veri kaybettirir. udisks işi bitirince (ya da hata verince) cevap verir.
   Eylemler bu yüzden yöntem zaman aşımı olmayan ayrı bir bus bağlantısından gider (izleme
   bağlantısının 5 sn `method_timeout`'u eylemlere uygulanmaz; ilk sürümde `Mount`'taki polkit
   istemi de bu 5 sn'ye takılırdı); `Mount`, `Lock`, `PowerOff`, `Eject` 120 sn ile sınırlı.
5b. **Sürücü başına tek çıkarma** (2026-10-13, inceleme). Düğme + `msg` ya da çift
   basış aynı sürücüye iki paralel görev açıp ikincisinde sahte `NotMounted` hatası üretiyordu.
   Servis çıkarılmakta olan sürücüleri tutar; süren çıkarma varken aynı sürücüye gelen istek
   loglanıp yok sayılır (bildirim yok; ilki sonucunu bildirir). Bitince yeni istek kabul edilir.
6. **Yetki** udisks'in polkit kurallarıyla; seçenekler boş (`auth.no_user_interaction` yok), yetki
   gerekirse Nuxut polkit ajanı sorar. Kullanıcı istemi kapatırsa (`NotAuthorizedDismissed`)
   bildirim gösterilmez. Eylem zaman aşımı 120 sn (istem dahil; `Unmount` hariç, bkz. 5a).
7. **Açma**: bağlama noktası `xdg-open` ile (ekran görüntüsü "Aç"ıyla aynı ayrık başlatma,
   `services::screenshot::save::open_path`).
8. **Komut adı `removable`** (`msg removable list|open|eject`): `media` MPRIS denetimine
   (`services::media`) ayrılmış kalsın diye.

## Config

```toml
[removable_media]
enabled = true   # false: udisks'e hiç bağlanılmaz (canlı; yeniden yüklemede servis durur/başlar)
notify = true    # takılınca bildirim
```

## Açık noktalar

- Hızlı panel listesi (takılı birimler, bağla/çıkar) — v1 dışı.
- Kilitli kripto kabı için kilit açma istemi (parola diyaloğu; `Encrypted.Unlock`).
- `automount` seçeneği (karar 2).
- Çok bölümlü bellekte bölüm başına bildirim yerine sürücü başına tek bildirim.
- `open_path` ekran görüntüsü modülünde; üçüncü kullanıcı gelirse ortak bir yardımcıya çıkar.
- Canlı doğrulama sahte udisks ile yapıldı; gerçek USB bellekle (polkit istemi, `PowerOff`) kullanıcı
  makinesinde elle doğrulanmalı (VM'de takılabilir aygıt yok).
