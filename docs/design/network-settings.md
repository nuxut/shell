# Ağ (Wi-Fi / VPN) ve Bluetooth: paylaşılan istemciler ve Ayarlar sayfaları

Taslak 2026-10-13 (görev t114/t115). Onay bekleyenler: crate yeri,
kabuktan çıkarma ve kayıt satırları, Ayarlar sayfaları.
Kararlar onaylandıkça bu notta tarihle işaretlenir.

## Bugün

- `src/services/network/` (~1700 satır): NetworkManager istemcisi — ObjectManager durumu,
  Wi-Fi aç/kapa, istek üzerine tarama, erişim noktaları, bağlan/kopar/unut, kayıtlı bağlantılar,
  VPN aç/kapa, gizli bilgi ajanı (`SecretAgent`, kullanıcı başına tek kimlik). Testler:
  `tests/it/network_service.rs` (sahte NM).
- `src/services/bluetooth/` (~1050 satır): BlueZ istemcisi + eşleştirme ajanı (`Agent1`,
  varsayılan ajan). Testler: `tests/it/bluetooth_service.rs`.
- İkisi de `src/services/dbus_util.rs`'e (ServiceThread, ObjectStore, NameOwner, Backoff,
  değer ayrıştırıcıları; 14 dosya kullanıyor) ve hata metinleri için `nuxut::i18n::t`'ye bağlı.
- Ayarlar'da ağ/Bluetooth sayfası yok; Ayarlar'ın NM/BlueZ kodu yok.

## Sorun

Ayarlar sayfaları hızlı panelin yaptığının üstüne çok daha fazlasını ister (bağlantı düzenleme,
gizli ağ, 802.1X, VPN içe aktarma, cihaz adı/görünürlük). Kodu Ayarlar'a kopyalamak iki istemci,
iki hata kümesi demek; Ayarlar'ın kabuğa `msg` üzerinden konuşması (Ekranlar sayfası gibi) ise
canlı tarama listesi ve ayrıntılı bağlantı ayarları için hem yavaş hem kabuk kapalıyken çalışmaz.

## Öneri

### 1. Paylaşılan crate'ler (shell deposunda)

```
shell/crates/nuxut-dbus     dbus_util'in genel kısmı (ServiceThread, ObjectStore, NameOwner,
                            Backoff, connect/close, değer ayrıştırıcıları)
shell/crates/nuxut-nm       network/{model,mod}.rs + yeni yazma API'leri (aşağıda)
shell/crates/nuxut-bluez    bluetooth/{model,mod}.rs + adaptör ayarları
```

- **Yer:** shell deposu, `crates/` (emsal: `crates/nuxut-pam`; takvimde `nuxut-calendar-ipc`).
  Ayarlar `path = "../shell/crates/nuxut-nm"` ile bağlanır (bugünkü `nuxut-calendar-ipc` gibi).
  Framework'e girmez: framework UI çerçevesidir, sistem servis istemcileri UI'sızdır ve zbus'ı
  framework'ün bağımlılığı yapmak gereksiz. (framework görüşü bekleniyor.)
- **UI'dan bağımsız:** crate'ler `nuxut::i18n` kullanmaz; hatalar tür olarak döner
  (`NmError::{Busy, AuthFailed, NoSecrets, NotFound, Dbus(String)…}`), metni çağıran çevirir.
  Böylece crate'ler framework'e de bağlı olmaz.
- **Aynı sürümler:** zbus 5.16, async-io yürütücüsü (shell ve settings zaten aynı).
- **Davranış değişmeden çıkarma:** ilk adım saf taşıma — shell `services::network` ince bir
  yeniden dışa aktarma olur, testler crate'e taşınır, kabuk tarafında satır değişikliği yalnızca
  `use` yolları + i18n eşlemesi.

### 2. Gizli bilgi ve eşleştirme ajanı tek: kabuk

NM ajan kimliği kullanıcı başına tektir. **Ayarlar ajan kaydetmez** (onay,
2026-10-13). Ayarlar bir bağlantıyı etkinleştirip NM şifre isterse kabuğun ajanı sorar (polkit
diyaloğuyla aynı ilke: kimlik bilgisi istemi kabuğun). Yeni bağlantıda kullanıcının yazdığı
şifre `AddAndActivateConnection` / `AddConnection` ayarlarına girer (NM saklar; ajan gerekmez).

- Ayarlar kabuğun ajanının varlığını **tahmin etmez**: etkinleştirme `NmError::NoSecrets` ya da
  `AuthFailed` dönünce "şifreyi güncelle" alanını açar (`Update` ile yazar). NM'de "ajan kayıtlı
  mı" sorusu yok; crate böyle bir bilgi vermez.
- Bluetooth `Pair` her zaman denenir; sonuç türleri ayrı: `AuthenticationRejected`,
  `AuthenticationFailed`, `AuthenticationCanceled`, `AuthenticationTimeout`, `ConnectionAttemptFailed`
  … Ayarlar kabuk kapalıysa (`msg ping`) "eşleştirme onayı gösterilemedi; Nuxut kabuğu çalışmıyor"
  der. Ajansız cihaz tahmini yok.

### 3. `nuxut-nm` yeni yazma API'leri

| API | NM çağrısı | Aşama |
|---|---|---|
| `connect_hidden(ssid, security, secret)` | `AddAndActivateConnection` (802-11-wireless.hidden=true) | v1 |
| `set_autoconnect`, `set_metered` | `Update` yaması (`GetSettings` + değişen alan) | v1 |
| `forget(path)` | `Delete` (var) | v1 |
| `import_wireguard(text)` | WireGuard `.conf` kendi ayrıştırıcımızla → `AddConnection` (type wireguard; dış araç yok) | v1 |
| `delete_vpn`, VPN aç/kapa | `Delete`, `ActivateConnection`/`DeactivateConnection` (var) | v1 |
| `device_details` | Ip4Config/Ip6Config, HwAddress, hız (salt okunur) | v1 |
| `connect_enterprise(ap, Eap{peap/ttls/tls, identity, anon, ca_cert, phase2, secret})` | `AddAndActivateConnection` (802-1x) | v2 |
| IPv4/IPv6 elle/DHCP, DNS, `set_mac_randomization` | `Update` yaması | v2 |
| `import_vpn(file)` (OpenVPN vb.) | `nmcli connection import type <t> file <f>` (D-Bus'ta içe aktarma yok; libnm FFI'ye değmez; NM eklenti paketi gerekir) | v2 |

- **Gizli bilgiler:** `Secret` (zeroize, Debug'da gizli); crate'ten loglanmaz. API şifreyi
  framework `TextInput` parola kipinin `SecretBuffer`'ından **String'e kopyalamadan** alır
  (`Secret::from_secret_buffer` ya da `&SecretBuffer` kabul eden yapıcı; ara kopya zeroize).
  *Aşama 2:* `Secret::from_bytes(SecretBuffer::to_zeroizing())` (kopyasız sahiplenme; aşağıda Kararlar).
- **Canlı bildirim:** crate değişikliği çağıranın olay döngüsüne `notify: Fn() + Send` kancasıyla
  haber verir (Ayarlar'ın timedate/wallpaper kalıbı); kabuk bugünkü olay kanalını kullanmaya devam
  eder. İkisi de aynı `ServiceThread` üstünde. *(Aşama 2: ayrı kanca yok, `sink` yeterli —
  Kararlar.)*
- **Hatalar** tür olarak (`NmError`, `BluezError`); metin çağıranda çevrilir (crate'te i18n yok).

### 4. `nuxut-bluez` ek API'leri

Adaptör `Alias` (ad), `Discoverable` + `DiscoverableTimeout`, `Pairable`; cihaz `Trusted`,
`Blocked`, `Alias`; `RemoveDevice` (var). `Pair` hata türleri yukarıda.

**t115 Bluetooth sayfası istekleri (2026-10-13).** Ses profili v2.

| # | İstek | Durum / karşılık |
|---|---|---|
| 1 | Pil (`Battery1.Percentage`, varsa; `PropertiesChanged`) | var: `Device::battery` |
| 2 | Ham `Icon` + `Class` (+ `Appearance`); ikon eşlemesi Ayarlar'da | `Device::{icon, class, appearance}` |
| 3 | Bağlan/kopar, unut (`RemoveDevice`), güven/engel/ad yazma | var |
| 4 | `StartDiscovery`/`StopDiscovery` + `SetDiscoveryFilter` (`Transport=auto`, `DuplicateData=false`); kendi başlattığı tarama ayrımı | filtre her başlatmada; `BluetoothState::own_discovery` (bizim oturum) vs `Adapter::discovering` (herhangi biri) |
| 5 | RSSI; `Name` ile `Alias` ayrımı (adsız gizlenebilsin) | `Device::{alias, remote_name}` (`remote_name: None` → BlueZ ad bilmiyor) |
| 6 | Adaptör `Powered` yazma; rfkill engeli ayrı hata türü (yumuşak/sert); çok adaptör (v1 ilki) | `BluezError::RfkillBlocked`, `Adapter::power_state`, `BluetoothState::rfkill` |
| 7 | `Pair` hata türleri + `CancelPairing`; ajan yok ayrı tür (ayırt edilebiliyorsa) | `cancel_pairing`; ajan yok ayırt **edilemez** (aşağıda) |
| 8 | `Connected` + `ServicesResolved` | `Device::{connected, services_resolved}` |
| 9 | Hepsi `notify` (sink) ile; sahte BlueZ pil, tür, rfkill, eşleştirme başarı/ret/iptal sunar | sahte BlueZ genişletildi |

Kararlar (2026-10-14; kaynak: BlueZ `doc/org.bluez.Adapter.rst`, `src/adapter.c`):

- **rfkill:** iki kaynak birlikte. *Engel var mı* BlueZ'den: `Powered=true` yazımı
  `org.bluez.Error.Blocked` döner (adapter.c `property_set_mode_complete`:
  `MGMT_STATUS_RFKILLED` → `.Blocked`, güç durumu `OFF_BLOCKED`) ve `Adapter1.PowerState`
  `off-blocked` olur (BlueZ ≥ 5.67, deneysel). *Yumuşak mı sert mi* yalnız çekirdekten bilinir:
  `/sys/class/rfkill/*/{type,soft,hard}` (`type=bluetooth`) salt okunur okunur
  (`BluetoothOptions::rfkill_dir`, testte geçici dizin); yalnız bir adaptör engelliyken ya da
  `Blocked` hatasında okunur, boşta okuma yok. Hata türü `BluezError::RfkillBlocked { hard:
  Option<bool> }` (`None`: sysfs okunamadı); durum `BluetoothState::rfkill`. BlueZ adı başka
  gelse bile `Powered` yazımı başarısızken `PowerState=off-blocked` ise rfkill sayılır.
- **Kendi taramamız:** BlueZ tarama oturumunu istemci başına tutar (adapter.c
  `discovery_list`, sahibi D-Bus göndereni); `Discovering` herhangi bir istemcide doğrudur.
  `BluetoothState::own_discovery` yalnız bizim `StartDiscovery`'miz sürerken doğru; başlatma
  başarısızsa (`InProgress`/`NotReady`) iner. Filtre (`SetDiscoveryFilter`) da istemci başınadır,
  her başlatmadan önce yeniden gönderilir.
- **Ajan yok ayırt edilemez:** BlueZ `Pair`'de çağıranın ajanı yoksa **varsayılan ajanı**
  (kabuğunki) kullanır; o da yoksa `NoInputNoOutput` ("just works") dener ve olası hata
  `AuthenticationFailed`/`AuthenticationRejected` olarak gelir — ayrı bir "ajan yok" hatası
  yok. Bu yüzden `NoAgent` türü eklenmedi; Ayarlar kabuk kapalıysa (`msg ping`) bunu kendi
  söyler (§2).
- **Ad:** `Device::alias` (BlueZ hep doldurur; yoksa adresten türetir) ve
  `Device::remote_name` (`Name`; yoksa `None` → tarama listesinde gizlenebilir); `name`
  gösterim için `alias` → `remote_name` → adres.
- **Kullanılamama nedeni** (istek, 2026-10-14): `NetworkState::unavailable`
  ve `BluetoothState::unavailable: Option<Unavailable>` (`nuxut_dbus::Unavailable`, iki istemcide
  aynı tür): `BusUnreachable` (bus'a bağlanılamadı/koptu; yeniden denemelerde **bir kez**
  yayınlanır), `ServiceNotRunning` (bus var, NM/BlueZ adının sahibi yok); `available == true`
  iken ve `Default`'ta `None`. Kabuk yalnız `available`'a bakar; davranış aynı. Testler:
  geçersiz bus adresi → tek `BusUnreachable`; bus var servis yok → `ServiceNotRunning`; servis
  gelince `None` (NM ve BlueZ).
- **Ürün yapısında özelliksiz `Bus::from_env` yok** (öneri reddedildi,
  2026-10-14; şart): NM istemcisi sır taşır, ortamla oturum bus'ına
  yönlenip sahte NM'e sır verilmesin. Ayarlar geliştirme/demo için kendi varsayılan olmayan
  özelliğiyle `nuxut-nm/test-support` ve `nuxut-bluez/test-support`'u açar.
- **Sahte sınırı:** zbus özellik yazıcısı yalnız `fdo` hatası dönebildiğinden sahte BlueZ
  rfkill'de `Blocked` yerine `Failed` döner; istemci bunu `PowerState=off-blocked` ile tanır
  (ad eşlemesi birim testinde). Sahte, adı bilinmeyen cihazda `Name`'i boş dize verir (istemci
  boşu yok sayar).

### 5. Sahte servisler

Crate'ler `examples/fake_nm` ve `examples/fake_bluez` sunar: oturum bus'ında çalışan sahte NM /
BlueZ. İstemci test anahtarıyla (`NUXUT_NM_BUS=session`, `NUXUT_BLUEZ_BUS=session`; yalnız
`dbus-run-session` içinde) oturum bus'ına bağlanır. Ayarlar'ın canlı testleri de bunları kullanır;
testte gerçek sistem NM'ine/BlueZ'ine hiçbir çağrı gitmez.

### 6. Ayarlar sayfaları

**Gezinme** (karar, 2026-10-13): kenar çubuğunda yeni **"Bağlantılar"** grubu,
Masaüstü ile Sistem arasında; sıra Wi-Fi, VPN, Bluetooth (ileride Paylaşım, Yazıcılar). Gerekçe:
Sistem grubu uzadı; bağlantılar ayrı bir zihinsel küme.

**Kapsam** (şart, onay, 2026-10-13):

- **v1 (t114):** Wi-Fi aç/kapa; ağ listesi (tarama yalnız sayfa görünürken); satır içi şifreyle
  bağlan; gizli ağ; kayıtlı ağ ayrıntısı: otomatik bağlan, ölçülü, unut. VPN listesi, aç/kapa,
  WireGuard içe aktarma, sil. Ethernet ayrıntısı salt okunur (IP, DNS, hız).
- **v2:** 802.1X formu, elle IPv4/IPv6/DNS, MAC rastgeleleştirme, OpenVPN içe aktarma (nmcli).
  Büyük formlar framework'te doğrulamalı alan ister; v1 bunları beklemez.
- **Bluetooth (t115)** Wi-Fi/VPN v1'den sonra: aç/kapa, cihaz adı, görünürlük (süreli; alt metin
  zamanlayıcıyla güncellenir, ayrı bileşen gerekmez), eşleşmiş cihazlar (bağlan/kopar, güven,
  engelle, unut), tarama listesi (sayfa görünürken). Sayfanın crate istekleri (pil, ham ikon/
  sınıf, tarama filtresi ve kendi tarama ayrımı, ad/takma ad ayrımı, rfkill, `CancelPairing`,
  `ServicesResolved`): §4 tablosu (2026-10-13). Ses profili v2.

**Framework** (Ayarlar doğrulaması): sinyal ikonu (`IconName::Wifi1–4`, `WifiLock`) ve
parola alanı (`TextInput` parola kipi) VAR. Eksik: dosya seçici portal istemcisi (VPN içe aktarma;
v1 için tek talep, framework'e), `TextInput` genel doğrulama/hata durumu (v2).

**Sayfa e2e eksikleri (2026-10-14; settings abbb1ca).**

| # | Crate | İstek |
|---|---|---|
| 1 | nuxut-nm | Kayıtlı Wi-Fi şifresini güncelleme: `Update` ile yalnız `802-11-wireless-security` şifresi (diğer alanlar korunur); `SecretBuffer` girişi kopyasız |
| 2 | nuxut-nm | polkit reddi tür olarak (`…PermissionDenied` → `NmError::Denied`) |
| 3 | nuxut-nm | bağlan/etkinleştir/kapat/unut/Wi-Fi aç-kapa için istek kimlikli sonuç olayı; yeni durum sonuçtan önce |
| 4 | nuxut-nm | `import_wireguard` yok sayılan anahtarları **adlarıyla** bildirsin (değer asla) |
| 5 | nuxut-nm | sahte NM: sahne seçimi (`NUXUT_FAKE_NM_SCENE`), yeni bağlantı/VPN gerçekten etkinleşir, yanlış şifre yalnız "Ev" |
| 6 | nuxut-bluez | eşleştirme başarı olayı (istek kimlikli) |
| 7 | nuxut-bluez | "ajan yok" ayrı hata (ayırt edilebiliyorsa) |
| 8 | nuxut-bluez | sahte BlueZ: RSSI, `Pair` → `Paired=true`, `RemoveDevice` cihazı siler, "Saat" sınıfı giyilebilir, sahne seçimi |

Kararlar (2026-10-14):

- **(1) Şifre güncelleme:** `NetworkService::set_wifi_password(conn, Secret)` — `GetSettings` +
  `802-11-wireless-security.psk` + `psk-flags=0` + `Update`; diğer alanlar `GetSettings`'ten
  aynen (otomatik bağlanma, ölçülü…). Şifre `valid_secret` ile denetlenir. Şifre değeri
  `Value::Str` olarak `Secret`'ten ödünç serileşir (kopya yok; giriş
  `Secret::from_bytes(SecretBuffer::to_zeroizing())`); `Command`/olay `Debug`'ında ve logda
  şifre yok. Gövdede gizli bilgi olduğundan NM kayıtlı gizli bilgileri birleştirmez
  (update_auth_cb); kişisel Wi-Fi'de tek gizli bilgi bu şifredir. **Kararlar (
  2026-10-14; net-reviewer şartı):**
  - Yalnız `key-mgmt` `wpa-psk` / `sae`. **WEP reddedilir** (`NmError::Unsupported`; WEP eski ve
    güvensiz, `wep-key0..3`/`wep-key-type` yarım güncelleme riski alınmaz); kurumsal, OWE, açık
    ağ da `Unsupported`. Reddedilende NM'ye `Update` gitmez.
  - `psk-flags` (`NMSettingSecretFlags`): `0` (sistem saklar) → aynen. `AGENT_OWNED` (1) ve/veya
    `NOT_REQUIRED` (4) → `0` yapılır: Nuxut'un kabuk ajanı `SaveSecrets` saklamaz, ajan sahipli
    şifre Nuxut'ta zaten kalıcı değil; şifre verildiğine göre de gerekli. `NOT_SAVED` (2; "her
    seferinde sor") kullanıcının bilinçli seçimi → **değiştirilmez**, `NmError::SecretNotSaved`;
    tanınmayan bit de aynı (anlamı bilinmeyen bayrak bozulmaz).
  - Kabuk anahtarları: `shell-network-error-unsupported`, `shell-network-error-secret-not-saved`
    (tr + en taslak; çeviri için işaretlendi).
  - (Önceki taslakta "`psk-flags` değiştirilmez; ajan sahipli şifreyi NM ajana kaydettirir"
    yazıyordu — Nuxut ajanı kaydetmediğinden şifre kaybolurdu; düzeltildi.)
- **(2) polkit reddi:** `NmError::Denied` — `org.freedesktop.NetworkManager*.PermissionDenied`
  adları (`NmError::from_dbus`; `From<zbus::Error>`). Kabuk anahtarı `shell-network-error-denied`.
- **(3) İstek kimliği:** eylem metotları `Option<RequestId>` döner (`None`: kuyruğa girmedi);
  sonuç `NetworkEvent::Done { request, action, object }` / `Failed { request, action, error }`.
  `Done`'dan önce o ana dek alınmış durum değişikliği `State` olarak yayınlanır (NM sinyalleri
  çağrı cevabından önce gelmişse yeni durum sonuçtan önce görünür; NM cevaptan sonra sinyal
  gönderirse durum sonra gelir — sahte NM'de yeni etkin nesne cevaptan önce eklenir). Tarama
  ve şifre cevabı sonuç olayı vermez. `connect` başarısında `object` kayıtlı bağlantı yolu.
- **(4) WireGuard yok sayılanlar:** `import_wireguard` → `WireGuardImport { request, ignored }`;
  `ignored` yalnız **adlar** (`PostUp`, `Table`, …; tekrarsız, ilk görülme sırası).
- **(5) Sahte NM:** sahne `NUXUT_FAKE_NM_SCENE` (`default`, `no-device`, `empty`, `wifi-off`,
  `basic`; bilinmeyen değer çıkış 2); `ActivateConnection`/`AddAndActivateConnection` gerçek etkin
  nesne oluşturur, `DeactivateConnection` kaldırır; "Ev" yanlış kayıtlı şifreyle (`ev-sifre-123`
  dışı) `FAILED`/`NO_SECRETS`; `set_deny` ile `Delete`/`Update` polkit reddi. Demo'da ayrıca
  `psk-flags` 1 / 2 / 4 bağlantıları ("Ajanli", "Sormali", "Gereksiz") ve WEP "EskiWep"
  (`wep-key0..3` kayıtlı).
- **Sahte servis koruması:** `fake_bus_guard` `unix:runtime=yes`'i de reddeder (=
  `$XDG_RUNTIME_DIR/bus`, kullanıcının oturum bus'ı; net-reviewer 2026-10-14).
- **Kırıcı API (crate sözleşmesi; settings ile aynı anda iniş):** eylem metotlarının dönüşü
  `bool` → `Option<RequestId>`; `NetworkEvent::Done`/`Failed`'a `request` alanı (alan
  adlandıran desenler `..` ister); `import_wireguard` dönüşü `Result<bool,_>` →
  `Result<WireGuardImport,_>`; `NmError::Denied` yeni kol (kapsamlı `match`).
- **(6) BlueZ istek kimliği:** `RequestId` artık `nuxut_dbus`'ta (iki istemci aynı tür, yeniden
  dışa aktarılır). `BluetoothService` eylemleri `Option<RequestId>` döner; sonuç
  `BluetoothEvent::Done { request, device, action }` / `Failed { request, device, action, error }`.
  `pair` başarısı: eşleşme kurulunca `Done { action: "pair" }`; ardından otomatik güven + bağlan
  adımları `request: None` (başarıları olay vermez, hataları `Failed { request: None }`). Önce
  bekleyen durum (`Paired=true`) yayınlanır. Tarama başlat/durdur da `Done` verir; süre dolunca
  kendiliğinden durdurma vermez.
- **(7) "Ajan yok" yine ayırt edilemez** (2026-10-14 yeniden bakıldı): BlueZ `AgentManager1`'de
  kayıtlı/varsayılan ajanı sorgulayan yöntem yok; `Pair` çağıranın ajanı yoksa varsayılan ajana,
  o da yoksa `NoInputNoOutput`'a düşer, ayrı hata adı yok. Kabuğun ajanı sistem bus'ında yalnız
  benzersiz adlı bağlantıda yayınlanır; iyi bilinen ad almak (`NameHasOwner` ile sorgulanabilir
  olmak) sistem bus'ında D-Bus politika dosyası (paket, `/usr/share/dbus-1/system.d`) ister.
  Uydurma tür eklenmedi. Seçenekler: (a) kabuk IPC'sine BlueZ ajanı
  durumu sorgusu (`msg bluetooth agent` → kayıtlı/değil; Ayarlar zaten `msg ping` kullanıyor),
  (b) paketle sistem bus politikası + iyi bilinen ad.
  **Karar (a)** (2026-10-14; politika dosyası yok): `nuxut-shell msg
  bluetooth agent-status` → tek satır JSON `{"registered":bool,"default":bool}` (çevrilmez,
  ADR-0010). `registered`: kabuğun ajanı `RegisterAgent` ile kayıtlı; `default`:
  `RequestDefaultAgent` başarılı. Kaynak `BluetoothState::agent` (`AgentStatus`; ajansız
  istemcide ve BlueZ yokken ikisi `false`; BlueZ yeniden başlayınca yeniden kaydedilir).
  Bluetooth servisi yoksa ya da örnek ajansızsa ikisi `false`. **Ayarlar davranışı:** `Pair`
  öncesi sorar; `default` değilse "Eşleştirme onayı için kabuk çalışmalı" uyarısı gösterir ve
  yine de `Pair` dener; yanıt gelmezse (kabuk kapalı/eski) eski davranış.
- **(8) Sahte BlueZ:** RSSI (cihaz başına), `Pair` → `Paired=true` (cevaptan önce sinyal),
  `RemoveDevice` nesneyi siler (pil dahil), "Saat" CoD `0x000704` (giyilebilir/kol saati) +
  görünüm `0x00c2`; sahne `NUXUT_FAKE_BLUEZ_SCENE` (`default`, `no-adapter`, `off`, `rfkill`,
  `empty`, `basic`).
- **Kırıcı API (BlueZ):** eylem dönüşleri `bool` → `Option<RequestId>`; `BluetoothEvent::Failed`'e
  `request` alanı; yeni `BluetoothEvent::Done` kolu (kapsamlı `match`).
- **Kırıcı API 6 (son kez; 2026-10-14):** gelecekte kol alabilecek public enum'lar
  `#[non_exhaustive]`: `NetworkEvent`, `NmError`, `ActivationFailure`, `SecretKind`,
  `ConnectionKind`, `DeviceKind`, `WireGuardErrorKind`, `BluetoothEvent`, `BluezError`,
  `PairingKind`, `PowerState`, `nuxut_dbus::Unavailable`. Dış kapsamlı `match`'ler `_ =>` kolu
  ister (kabukta eklendi: olaylarda `false`, hatalarda genel satır + `Display`, eşleştirmede
  kartın tanımadığı tür beklemeden reddedilir). Eklenmeyenler: `Security` (yeni güvenlik türü
  UI'da bilinçli karar ister; kırılması istenir), NM'nin sabit enum aynaları (`NmState`,
  `Connectivity`, `ActiveState`, `Metered`), `BusKind`, `PairingReply` (çağıran kurar);
  struct'lar (Ayarlar test literalleri kurar).

## Aşamalar

1. **Çıkarma** (shell review): `nuxut-dbus`, `nuxut-nm`, `nuxut-bluez`;
   davranış değişmez; testler taşınır ve geçer.
2. **Yazma API'leri v1**: tablo 3'ün v1 satırları, `notify` kancası,
   `SecretBuffer` girişi, `Pair` hata türleri, sahte servisler.
3. **Ayarlar sayfaları v1**: Wi-Fi + VPN (t114), sonra Bluetooth (t115).
4. Hızlı panel: "Gizli ağ…" ve VPN içe aktarma Ayarlar sayfasını açar (kopya UI yok).
5. v2 API'leri + formları, framework doğrulamalı alan gelince.

## Kararlar (2026-10-13)

- **Crate sözleşmesi** (2026-10-14; 62c67a6 settings main'i kırdı):
  shell/crates/* public API'sinde kırıcı değişiklik iniş ÖNCESİ settings'te derlenir (`cargo check
  --all-targets`, settings main'i dala karşı) ve settings düzeltmesi aynı dakikada iner; Ayarlar'la
  önceden koordine. Olay/hata enum'ları `#[non_exhaustive]`: **yeni kol kırıcı sayılmaz**
  (2026-10-14); yeni alan, dönüş türü değişikliği ya da kaldırma kırıcıdır.

- **Crate yeri:** `shell/crates/{nuxut-dbus,nuxut-nm,nuxut-bluez}` (onay); framework'e
  zbus girmez. Taşıma onaylı: taşıma commit'i yalnız taşıma (davranış değişmez, kabuk
  testleri önce = sonra), yazma API'leri ayrı commit'ler; iniş öncesi önceden haber.
- **Gizli bilgi girişi** (framework + Ayarlar): çağrı boyunca ödünç `&[u8]`; thread'e
  taşınması gerekiyorsa `zeroize::Zeroizing<Vec<u8>>`. Ayarlar `SecretBuffer::to_zeroizing()`
  (framework) ile tek kopya, ara `String` yok. **Bilinen sınır:** zbus'ın mesaj tamponu
  zeroize edilmez.
- **Ayarlar asla ajan olmaz** (2026-10-13; aşama 1 bulgusu): `BluetoothService` bugün
  her zaman `RegisterAgent` + `RequestDefaultAgent` yapıyor; Ayarlar başlatsa kabuğun varsayılan
  eşleştirme ajanını elinden alırdı. Aşama 2 şartı: `BluetoothOptions`'a ajan kaydetmeme seçeneği
  (NM'de `agent_id: None` var); kabuğa özgü sabitler (`AGENT_ID`, `agent_id_for`, test env)
  kabuk tarafına. Ayarlar her iki istemciyi de ajansız açar.
- **Dosya seçici:** framework'te ayrı, isteğe bağlı `nuxut-portal` crate'i (FileChooser
  OpenFile/SaveFile, iptal, filtreler, xdg-foreign ebeveyn; sonuç calloop olayı) — framework.

- **Aşama 1 yapıldı (2026-10-13):** yalnız taşıma, davranış değişmez.
  `dbus_util` → `nuxut-dbus` (kabuk modülü kaldırıldı, servisler doğrudan `nuxut_dbus::`);
  `network`/`bluetooth` → `nuxut-nm`/`nuxut-bluez`, kabukta `services/{network,bluetooth}.rs`
  ince yeniden dışa aktarma; hata→metin eşlemesi tek yerde, `system.rs` (şart). i18n: `Failed { message: String }` →
  `Failed { error: NmError | BluezError }`; metin kabukta aynı `shell-network-error-*` /
  `shell-bluetooth-error-*` anahtarlarıyla (`CallFailed(detail)` → `…-call-failed` + `detail`).
  Bluetooth log satırı artık çevrilmiş metin yerine `BluezError`'ın `Display`'i (yalnız log).
  Entegrasyon testleri crate'lerin `tests/`'ine taşındı; ortak `tests/support/` `#[path]` ile
  paylaşılır (kopya yok). Bilinen fark: tracing hedefi `nuxut_shell::services::network` →
  `nuxut_nm` (yalnız `RUST_LOG`'da hedef süzgeci kullananı etkiler). `NmError`/`BluezError`
  `Debug`'ı elle ve redakte: `CallFailed(<redacted>)` — metin SSID/bağlantı/cihaz adı
  içerebilir (nmx-reviewer, 2026-10-13).

- **Aşama 2 yapıldı (2026-10-13; inceleme bekliyor).** Kabuk
  davranışı değişmez (kabuk testleri aynı; yeni olayları yok sayar). Netleşen kararlar:
  - **Ajan seçeneği — güvenli varsayılan** (2026-10-13: *Ayarlar unutsa bile ajan
    olamaz*): `NetworkOptions::default()` ve `BluetoothOptions::default()` **ajansızdır**
    (`agent_id: None`, `agent_path: None` → ajan nesnesi yok, `Register` / `RegisterAgent` /
    `RequestDefaultAgent` yok). Kabuk ajanı açıkça ister: `NetworkOptions::with_agent(id)`
    (`agent_id_for` sonucu; test örneği yine kaydolmaz), `BluetoothOptions::with_agent()`
    (`AGENT_PATH`) — `src/system.rs` `network_options` / `bluetooth_options`; kabuk davranışı
    aynı, birim testi `shell_requests_agents_explicitly` + crate testleri (`with_agent` →
    kayıt; varsayılan → kayıt yok). Kabuğa özgü `agent_id_for` + `NUXUT_TEST_NM_AGENT` kabukta
    (`src/services/network.rs`); `AGENT_ID` / `AGENT_PATH` crate'te sabit olarak kaldı.
  - **Bus seçimi:** `Options::bus: BusKind` (varsayılan sistem). `nuxut_nm::bus_from_env()` /
    `nuxut_bluez::bus_from_env()` (`NUXUT_NM_BUS` / `NUXUT_BLUEZ_BUS=session`) ve
    `BusKind::from_env` **yalnız `test-support` cargo özelliğiyle** derlenir (şart,
    2026-10-13): üründe hiç yok; sahte servis/probe örnekleri `required-features`. Özellik yalnız
    kabuğun (ve ileride Ayarlar'ın) dev-dependency'sinde açılır; ürün derlemesinde
    `cargo tree -e normal,features` yalnız `default` gösterir. Kabuk bunu kullanmaz (her zaman
    sistem bus'ı): ortamı değişmiş bir kabuğun şifre ajanı sahte bir NM'ye şifre vermesin.
  - **Sahte servisler:** `crates/nuxut-nm/tests/fake_nm/` ve `crates/nuxut-bluez/tests/fake_bluez/`
    tek kod; testler `mod`, örnekler (`examples/fake_nm.rs`, `examples/fake_bluez.rs`) `#[path]` ile
    kullanır. Örnekler kullanıcının oturum bus'ında (`unix:path=/run/user/*/bus`) çalışmayı
    reddeder. **Review şartı 5 (2026-10-13):** koruma `nuxut_dbus::fake_bus_guard`
    (`test-support`) — ayrıca açık `NUXUT_FAKE_BUS=1` işareti şart (yanlış bus tespitine karşı
    ikinci kilit) ve `unix:abstract=` biçimli adres (dbus-launch/X oturumu) gerçek oturum
    sayılıp reddedilir; `$XDG_RUNTIME_DIR/bus` da red. Canlı test betikleri `dbus-run-session`
    içinde `NUXUT_FAKE_BUS=1` ayarlar (bu makinede `dbus-run-session` `unix:path=/tmp/dbus-*`
    verir). Uçtan uca istemciler `examples/nm_probe.rs`, `examples/bt_probe.rs` (ajansız,
    `…_BUS=session` zorunlu); `dbus-run-session` içinde denendi.
  - **Gizli bilgi:** `Secret::from_bytes(Zeroizing<Vec<u8>>)` (UTF-8 doğrulamalı, kopyasız
    sahiplenme; geçersizde tampon sıfırlanır) ve `Secret::from_utf8(&[u8])`. Kaldırılan kopyalar:
    yeni Wi-Fi ayarlarındaki `pw.expose().to_owned()` (artık `Value::Str` şifreden ödünç; ayarlar
    çağrı geleceğinin içinde kurulur) ve ajanın `GetSecrets` cevabındaki `OwnedValue` kopyası
    (cevap şifreden ödünç serileşen `SecretsReply`; tel biçimi `a{sa{sv}}` birim testiyle aynı).
    WireGuard özel/ön paylaşımlı anahtarları da `Secret` ve ödünç. Kalan sınır değişmedi: zbus
    mesaj tamponu (ve NM'nin kendi kopyası) sıfırlanmaz; `import_wireguard` ayrıştırıcısı anahtar
    başına bir sıfırlanan kopya yapar.
  - **Sonuç olayları:** aşama 2 eylemleri başarıda `NetworkEvent::Done { action, object }`
    (`connect_hidden` / `autoconnect` / `metered` / `import_wireguard`; `object` kayıtlı bağlantı
    yolu), hata `Failed`. `device_details` → `NetworkEvent::DeviceDetails`.
    `import_wireguard` ayrıştırmayı çağıranın thread'inde yapar: hatalı dosya anında
    `Err(WireGuardError { line, kind })` (değer içermez).
  - **"Şifreyi güncelle" sinyali:** NM `ActivateConnection`'ı kabul edip sonradan vazgeçer; bu
    yüzden `NmError::NoSecrets` yerine `NetworkEvent::ActivationFailed { connection, reason }`:
    cihaz `StateChanged` → `FAILED` + neden 7 (`NoSecrets`) / 8–11 (supplicant, `AuthFailed`);
    VPN etkin bağlantısı `DEACTIVATED` + 9/10. Kabuk bu olayı yok sayar (davranış aynı).
  - **`Update` yaması:** `GetSettings` (gizli bilgisiz) + `connection.autoconnect` /
    `connection.metered` + `Update`. **NM kaynağıyla doğrulandı** (istek,
    2026-10-13; gerçek NM'ye çağrı yok): NetworkManager 1.58.0
    `src/core/settings/nm-settings-connection.c`, `update_auth_cb()` — `Update`, `UpdateUnsaved`,
    `Save` ve `Update2` hepsi `settings_connection_update()` → polkit → `update_auth_cb()`:
    ```c
    if (!_nm_connection_aggregate(info->new_settings, NM_CONNECTION_AGGREGATE_ANY_SECRETS, NULL)) {
        /* If the new connection has no secrets, we do not want to remove all
         * secrets, rather we keep all the existing ones. Do that by merging
         * them in to the new connection. */
        secrets = nm_connection_to_dbus(nm_settings_connection_get_connection(self),
                                        NM_CONNECTION_SERIALIZE_WITH_SECRETS);
        nm_connection_update_secrets(info->new_settings, NULL, secrets, NULL);
        if (priv->agent_secrets) nm_connection_update_secrets(..., priv->agent_secrets, NULL);
    } else { update_agent_secrets_cache(self, info->new_settings); }
    ```
    (https://github.com/NetworkManager/NetworkManager/blob/1.58.0/src/core/settings/nm-settings-connection.c;
    `main` aynı). Sonuç: gizli bilgi **hiç** içermeyen `Update` kayıtlı sistem gizli bilgilerinin
    (ve ajan önbelleğinin) **tümünü** korur; tek bir gizli bilgi bile gönderilirse birleştirme
    yapılmaz. `GetSettings` gizli bilgi vermediğinden yamamız hep ilk dala düşer: `Update`
    güvenli, `Update2`'ye gerek yok (`Update2` bayrakları yalnız kalıcılık/`VOLATILE`/
    `BLOCK_AUTOCONNECT`/`NO_REAPPLY`; bu davranışı değiştirmez). Sahte NM aynısını yapar (gizli
    bilgi yoksa hepsini birleştir, varsa birleştirme yok). Test: yalnız ilgili alan değişir, psk
    gönderilmez (`secrets=0`) ve kalır.
  - **WireGuard:** kendi ayrıştırıcımız (`nuxut_nm::wireguard`): `[Interface]` PrivateKey/Address/
    DNS (IP olmayanlar arama alanı)/ListenPort/FwMark/MTU, `[Peer]` PublicKey/PresharedKey/
    AllowedIPs/Endpoint/PersistentKeepalive; anahtar 32 baytlık base64 denetimi; wg-quick'e özgü
    `Table`/`PreUp`/`PostUp`/`PreDown`/`PostDown`/`SaveConfig` **çalıştırılmaz, içe aktarılmaz**
    (`WireGuardConfig::ignored` → Ayarlar uyarabilir); bilinmeyen anahtar/bölüm hata. NM ayarı:
    `type wireguard`, ad = bağlantı + arayüz adı (1–15 bayt), `autoconnect=false`, adres yoksa
    `ipv4/ipv6.method=disabled`. Silme: `forget` (her kayıtlı bağlantı, VPN dahil).
    **Anahtarlar NM kaynağıyla doğrulandı** (1.58.0): `src/libnm-core-public/nm-setting-wireguard.h`
    — `wireguard.{private-key, private-key-flags, listen-port, fwmark, mtu, peers, peer-routes}`;
    eş anahtarları `public-key, preshared-key, preshared-key-flags, allowed-ips, endpoint,
    persistent-keepalive`. `src/libnm-core-impl/nm-setting-wireguard.c` `peers_from_dbus()` türleri:
    `public-key`/`endpoint`/`preshared-key` `s`, `preshared-key-flags`/`persistent-keepalive` `u`,
    `allowed-ips` `as`; zorunlu yalnız `public-key` (yoksa katı kipte hata, değilse eş atlanır) —
    ayrıştırıcımız `PublicKey`'siz eşi zaten reddeder. D-Bus ayar belgesi
    (networkmanager.dev/docs/api/latest/nm-settings-dbus.html): `private-key` `s`, `listen-port` /
    `fwmark` / `mtu` `u`, `peers` `aa{sv}`; `ipv4.address-data` `aa{sv}` (`address`, `prefix`),
    `ipv4.dns` `au` (ağ bayt sırası; belgede `dns-data` `as` lehine *deprecated* ama kabul ediliyor —
    eski NM'lerle uyum için `au`/`aay` bırakıldı), `ipv6.dns` `aay`, `dns-search` `as`,
    `method` `disabled`/`manual` (IPv4 ve IPv6), `connection.metered` `i`, `autoconnect` `b`.
    Gönderdiğimiz türler bunlarla birebir (birim testi `nm_settings_shape`, `peers` imzası `aa{sv}`).
  - **Cihazlar:** `NetworkState::devices` (döngü hariç: tür, arayüz, bağlı, etkin bağlantı);
    `device_details` IP4Config/IP6Config `AddressData`, ağ geçidi, DNS (`NameserverData` +
    IPv6 `Nameservers`), MAC (`HwAddress`), hız (kablolu `Speed`, Wi-Fi `Bitrate`).
    `SavedConnection` artık `autoconnect` ve `metered` taşır.
  - **BlueZ:** `set_adapter_alias`, `set_discoverable(adapter, on, timeout_secs)` (önce
    `DiscoverableTimeout`, sonra `Discoverable`), `set_pairable`, `set_blocked`,
    `set_device_alias`; `Adapter::{discoverable, discoverable_timeout}`. `BluezError` yeni türler:
    `AuthenticationRejected/Failed/Canceled/Timeout`, `ConnectionAttemptFailed`, `AlreadyExists`,
    `InProgress`, `NotReady`, `InvalidValue` (tüm eylemlerde `org.bluez.Error.*` adından).
    Kabuk bunları önceki gibi genel satırla gösterir (ayrıntı: D-Bus hata adı).
  - **`notify`:** ayrı kanca eklenmedi; `sink: Fn(Event) + Send` yeterli — Ayarlar kalıbı (sink →
    `Mutex<Vec<_>>` + döngüyü uyandır) crate belgelerinde örnekli.
  - Yeni crate yok (`zeroize`, `zbus` zaten bağımlılık).

## Açık sorular

- OpenVPN içe aktarma (v2) `nmcli`: `networkmanager` paketinde; eklenti paketleri
  (`networkmanager-openvpn` vb.) isteğe bağlı, DEPENDENCIES'e yazılır.
