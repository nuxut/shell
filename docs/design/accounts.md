# Hesaplar çekirdeği (taslak)

Durum: **taslak**, accounts-research, 2026-10-16. Araştırma ve kaynaklar:
`docs/research/online-accounts.md`. Kullanıcı hedefi (2026-10-16): tek "Hesap ekle" ile takvim,
kişiler, e-posta ve dosyalar masaüstüne bağlansın.

## 1. Öneri özeti

- **Kendi hesap servisimiz** (`nuxut-accountsd`, D-Bus `org.nuxut.Accounts1`) bütün hesapların ve
  belirteçlerin tek sahibi olur. Takvim, kişiler, posta ve dosya servisleri ondan kısa ömürlü
  erişim belirteci ya da parola ister. GOA'nın modeli bu, ama kendi istemci kimliklerimizle.
- **GOA köprüsü: ürün yolu olarak hayır.**
  1. GOA Google Drive'ı artık vermiyor (3.58 / GNOME 50). Köprünün ana gerekçesi ortadan kalktı.
  2. GOA'ya ekleme GTK4 + libadwaita (`libgoa-backend`) ya da Mint'in aracını gerektiriyor;
     Nuxut'un görsel dili ve bağımlılık ilkesiyle çelişiyor.
  3. GNOME geliştiricileri GNOME'un anahtarlarının GNOME dışı kullanımını "sorunlu" buluyor.
     Google bir istemciyi engellediğinde (KDE, 2024-11) çaresiz kalırız.
  4. GOA'nın M365 istemcisi doğrulanmamış.

  Tek kazanç Gmail olurdu. Gmail için yol kendi istemcimizle kısıtlı kapsam doğrulaması: veri
  yalnızca cihazda kaldığı için Google'ın ifadesine göre CASA gerekmiyor.
- **İsteğe bağlı küçük adaptör (Faz 5, kullanıcı kararı K4):** "GNOME hesaplarını içe al".
  Kullanıcının **zaten** GOA'da hesabı varsa (ör. GNOME'dan geçiş) zbus ile salt okunur listeler
  ve Nuxut'ta kendi akışımızla yeniden ekletir. GOA belirteci kullanmaz; GTK bağımlılığı yok.

## 2. Sağlayıcı × hizmet matrisi

| Sağlayıcı | Kimlik | Takvim | Kişiler | Posta | Dosyalar |
|---|---|---|---|---|---|
| Google | OAuth (kendi Desktop istemcimiz) | ✅ Calendar v3 (var; hassas, ücretsiz doğrulama) | ✅ People API ya da CardDAV `contacts` (hassas) | ⚠️ IMAP XOAUTH2 `mail.google.com` (kısıtlı: inceleme, CASA yok, K1) | ⚠️ `drive` kısıtlı (K2); `drive.file` göz atma vermez |
| Microsoft (kişisel + iş) | OAuth (kendi Entra public istemcimiz) | ✅ Graph (var) | ✅ Graph `Contacts.ReadWrite` | ✅ IMAP/SMTP XOAUTH2 (`IMAP.AccessAsUser.All`, `SMTP.Send`) | ✅ Graph `Files.ReadWrite` (OneDrive) |
| Nextcloud | Login Flow v2 → uygulama parolası (var) | ✅ CalDAV (var) | ✅ CardDAV | — (Nextcloud Mail ayrı sunucu; genel IMAP ile) | ✅ WebDAV |
| iCloud | Apple ID + uygulamaya özel parola | ✅ CalDAV (var: `Caldav`) | ✅ CardDAV | ✅ IMAP/SMTP | ❌ resmî API yok; sunulmaz, metinle açıklanır |
| Genel | Parola / uygulama parolası | ✅ CalDAV | ✅ CardDAV | ✅ IMAP/SMTP (autoconfig) | ✅ WebDAV |
| ICS aboneliği | yok | ✅ salt okunur (var) | — | — | — |

İşaretler:
- ✅ kendi istemcimizle engelsiz.
- ⚠️ kullanıcı kararı ya da Google incelemesine bağlı. Karar verilene kadar seçenek görünmez
  (mevcut "kimliği yoksa gizle" kuralı).
- ❌ sunulmaz.

Microsoft iş hesaplarında yayımcı doğrulaması yoksa yönetici onayı gerekir (K3; mevcut
`oauth-admin-consent` akışı).

**Kapsam ilkesi:** hesap eklenirken yalnızca kullanıcının açtığı hizmetlerin kapsamları istenir
(artımlı onay). Google `include_granted_scopes=true` ile sonradan kapsam ekliyor; Microsoft
kapsamları istek başına alıyor. Böylece:
- Yalnız takvim isteyen biri posta izni ekranı görmez.
- Kısıtlı kapsam yalnızca "Posta" anahtarı açılınca istenir.

## 3. Süreçler ve sınırlar

```
Ayarlar ─┐                      ┌─ nuxut-calendard (Calendar1, var)
Shell ───┼─ org.nuxut.Accounts1 ┼─ nuxut-contactsd   (yeni; kişiler)
         │   nuxut-accountsd    ├─ nuxut-maild       (yeni; IMAP IDLE bildirimi)
         │   (hesap + belirteç) └─ nuxut-cloudfs     (yeni; FUSE dosyalar)
         └─ Secret Service (oo7) ← yalnızca accountsd
```

- **accountsd** hesap listesini (`$XDG_DATA_HOME/nuxut/accounts.json`, var) ve Secret Service'i
  tek başına okur ve yazar. OAuth akışları, yenileme (hesap başına tek uçuş), iptal ve sağlık
  durumu burada.
  - Ağ trafiği yalnızca kimlik uç noktalarına gider (nuxut-net sabit liste, ADR-0007 §6).
  - Ayrıştırdığı tek şey küçük JSON token yanıtları.
- **Tüketici daemon'lar** belirteci istek başına alır ve yalnız bellekte tutar (`SecretString`).
  - `NeedsReconnect` ya da `invalid_grant` gelince accountsd'ye bildirirler, accountsd hesabı
    işaretler.
  - Güvenilmeyen veriyi (iCalendar, vCard, MIME, WebDAV XML) yalnız onlar ayrıştırır.
  - Ayrıştırıcı yalıtımı framework ADR-0007 deseniyle (işçi süreç + seccomp + landlock)
    sonraki faz.
- **D-Bus etkinleştirme + systemd `--user`.** Boştayken 0 CPU, hedef RSS < 10 MB. Tokio yok
  (calendar ADR-0004); posta IDLE için ayrıntı K7.
- **Tüketici yetkisi:** oturum bus'ındaki her süreç `GetAccessToken` çağırabilir (GOA ile aynı).
  Aynı kullanıcı kimliğindeki kötü amaçlı süreç zaten Secret Service'i okuyabildiği için gerçek
  bir sınır değil. Yine de iki önlem alınır:
  - Çağrı, çağıranın unique name'i, PID'i ve `/proc/<pid>/exe` yoluyla loglanır (yalnız hesap
    kimliği; içerik yok).
  - Belirteç yalnızca hesapta **açık** olan hizmetin kapsamıyla verilir: dosyalar kapalıysa
    `Files.*` içeren belirteç hiç yoktur.

## 4. D-Bus sözleşmesi taslağı — `org.nuxut.Accounts1`

Nesne: `/org/nuxut/Accounts1`.

```
ListAccounts() → a(ssss a{sb} s)            # id, provider, display_name, login, services{calendar:true,…}, state
AvailableProviders() → a(s as)              # sağlayıcı + bu derlemede sunulabilen hizmetler
BeginAddAccount(s provider, as services, s reconnect_id) → s flow_id
FlowInput(s flow_id, a{sv} input)           # sunucu adresi, parola, "başka cihaz" seçimi
CancelFlow(s flow_id)
SetServiceEnabled(s id, s service, b on)    # yeni kapsam gerekiyorsa akış başlatır → flow_id döner
RemoveAccount(s id)                          # iptal + anahtarlık + tüketicilere AccountRemoved
GetAccessToken(s id, s service) → (s token, u expires_in)
GetPassword(s id, s service) → s             # parola tabanlı hesaplar
ServiceEndpoint(s id, s service) → a{sv}     # imap_host/port/tls, caldav_root, webdav_root…
ReportAuthFailure(s id, s service, s code)   # tüketici: 401/invalid_grant → NeedsReconnect

signal AccountsChanged()
signal FlowChanged(s flow_id, s state, a{sv} data)   # calendar FlowState ile aynı durumlar
signal AccountRemoved(s id)
```

- Hata anahtarları calendar ADR-0007 §8 tablosunun `accounts-` ad alanına taşınmış hâli.
  Kullanıcı metni katalogdan gelir (i18n ADR-0010).
- Sürümleme: `Accounts1` arayüzüne yalnız ekleme yapılır; kırıcı değişiklik `Accounts2` olur.

## 5. Ayarlar → "Hesaplar" akışları

Mevcut `settings/src/pages/accounts.rs` ilkesi korunur: her ekranda tek soru, tek büyük düğme.
Akış daemon'da yaşar, sayfa yalnız durumu çizer.

1. **Hesap ekle** → sağlayıcı ızgarası (Google, Microsoft, Nextcloud, iCloud, Diğer). Kimliği
   olmayan sağlayıcı görünmez.
2. **Hizmet seçimi:** sağlayıcının sunabildiği hizmetler, anahtar olarak. Varsayılanlar: takvim,
   kişiler, dosyalar açık; posta, kullanıcının sistemde bir posta işleyicisi varsa açık.
   Açıklamalar düz dille, teknik ad yok (settings-ux kararı).
3. **Kimlik:**
   - OAuth: tarayıcı açılır, "Tarayıcıda izin verin" ekranı ve "Başka cihazda giriş" (yalnız
     Microsoft).
   - Nextcloud: sunucu adresi → Login Flow.
   - iCloud: Apple ID + uygulamaya özel parola, appleid.apple.com bağlantısı ve iki satır
     açıklamayla.
   - Diğer: e-posta → autoconfig ya da SRV keşfi, bulunamazsa sunucu alanları.
4. **Keşif:** takvimler, adres defterleri, posta klasörleri ve dosya kökü bulunur. "Bitti"
   ekranı her hizmet için tek satır özet gösterir.
5. **Hesap ayrıntısı:**
   - Hizmet anahtarları. Kapsam gerekiyorsa açmak yeniden izin ister.
   - Durum satırı (yeniden bağlan, yönetici onayı, kota).
   - "Hesabı kaldır": Google'da iptal; Microsoft'ta "hesabınızdan da kaldırın" bağlantısı.
6. **Shell:** hesap durumu bozulunca tek bildirim ("Google hesabınızı yeniden bağlayın"),
   eylemi Ayarlar'ın ilgili sayfası.

## 6. Hizmetlerin masaüstüne bağlanması

- **Takvim:** `nuxut-calendard` (var). Belirteç ve parola accountsd'den gelir.
- **Kişiler:** `nuxut-contactsd`: CardDAV, Graph ve People senkronu; redb önbellek. Calendar'ın
  `calcard` (ADR-0003) ve redb (ADR-0002) seçimleri yeniden kullanılır.
  - Tüketiciler: başlatıcı araması, posta adres tamamlama, takvim katılımcıları.
  - Evolution ve Thunderbird için sonraki faz: yerel CardDAV sunmak yerine sistemdeki istemciye
    vCard dışa aktarma.
- **Posta:** Nuxut'un posta uygulaması yok (uygulama paketi fikri park edildi, m6 sonrası).
  Bu yüzden ilk sürüm:
  - `nuxut-maild`: hesap başına IMAP IDLE ile yeni ileti bildirimi ve okunmamış sayısı
    (shell'de rozet).
  - Bildirim eylemi kullanıcının varsayılan posta istemcisini açar.
  - Thunderbird ve Evolution belirteçlerimizi kullanamaz (kendi istemci kimlikleri var). Onlar
    için en fazla adres ve sunucu ön doldurması yapılabilir **(doğrulanmadı)**.
  - Gerçek "tam bağlı posta" kendi posta uygulamamızla gelir. Kullanıcıya bu sınır dürüstçe
    söylenir.
- **Dosyalar:** `nuxut-cloudfs`: Rust FUSE (`fuser` ya da `polyfuse`, K8), bağlama noktası
  `~/Bulut/<hesap>`.
  - İsteğe bağlı indirme; önbellek `$XDG_CACHE_HOME/nuxut/cloud/` altında, boyut üst sınırlı,
    LRU.
  - Klasör başına "Çevrimdışı tut".
  - Yazmalar kuyruğa alınır: Graph upload session, WebDAV PUT. Çakışmada "(çakışan kopya)" dosyası
    oluşturulur, veri asla ezilmez.
  - Uzak dosyalar için önizleme üretilmez; önbellekte olan için üretilir.
  - Dosya seçici ve gelecekteki dosya yöneticisi bağlama noktalarını kenar çubuğunda "Bulut"
    altında gösterir (`docs/design/file-chooser.md`'ye ek).
  - rclone reddedildi: Go dış ikili, belirteçler kendi yapılandırmasında (tek sahip ilkesini
    bozar), önbellek davranışı bizim denetimimizde değil.

## 7. Mevcut takvim hesap kodunun çekirdeğe taşınması

Bugün (calendar reposu):
- `crates/nuxut-accounts`: hesap modeli ve kalıcı liste, `KeyringStore` (oo7), OAuth (loopback
  + PKCE, cihaz kodu, yenileme, iptal, kapsam denetimi), Nextcloud Login Flow, tarayıcı açma,
  `client_ids`. Toplam ~2.5 bin satır, UI bilmez.
- Akışlar `nuxut-calendard` içinde (`flows.rs`, `engine.rs::auth_for`/`AccountTokens`).
- Ayarlar `org.nuxut.Calendar1.BeginAddAccount` ve `ListAccounts` ile konuşuyor.

Adımlar (davranış değişmeden):
1. `nuxut-accounts` crate'i olduğu gibi accounts reposuna (K6) taşınır.
2. `Provider` enum'u "sağlayıcı" ve "hizmet kümesi" olarak ayrılır:
   - `Caldav` → `Generic` (+ `services`). `accounts.json` sürüm 2'ye göç eder.
   - Eski `caldav` girdisi takvim hizmeti açık `generic` hesap olur.
   - Göç testli ve atomik.
3. `SecretKind` hizmet bilmez, aynı kalır: tek yenileme belirteci, tüm kapsamlar.
4. `flows.rs` + `AccountTokens` accountsd'ye taşınır. calendard'da `auth_for()`
   `Accounts1.GetAccessToken(id,"calendar")` çağrısına döner. Hesap başına tek uçuş kilidi
   accountsd'de kalır.
5. `Calendar1.BeginAddAccount`/`RemoveAccount` bir sürüm boyunca accountsd'ye yönlendiren
   uyumluluk katmanı olur, sonra kaldırılır. Ayarlar `Accounts1`'e geçer. `Calendar1`'de
   yalnız takvim görünürlüğü ve rengi kalır.
6. Anahtarlık öğe nitelikleri (`application=nuxut`, `nuxut-account=<id>`) değişmez. Mevcut
   hesaplar yeniden bağlanmadan çalışır.

## 8. Fazlar ve tahmini büyüklük

| Faz | İçerik | Büyüklük (satır, test dahil) |
|---|---|---|
| 1 | accountsd + `Accounts1` + taşıma (§7) + Ayarlar sayfasının `Accounts1`'e geçişi; hizmet anahtarları yalnız takvim | ~3–4 bin |
| 2 | Kişiler: contactsd (CardDAV, Graph, People) + başlatıcı araması | ~4–5 bin |
| 3 | Posta bildirimi: maild (IMAP IDLE + XOAUTH2, autoconfig) + shell rozeti; Microsoft, iCloud, genel; Google K1 sonrası | ~3–4 bin |
| 4 | Dosyalar: cloudfs (FUSE; Nextcloud/WebDAV + OneDrive) + dosya seçici "Bulut" | ~6–8 bin |
| 5 | İsteğe bağlı: GOA salt okunur içe aktarma (K4); ayrıştırıcı yalıtım işçileri | ~0.5 bin + işçi başına ~1 bin |

Her yeni crate (`fuser`/`polyfuse`, IMAP ve MIME crate'leri) `docs/DEPENDENCIES.md`'ye gerekçe
ve Arch çalışma zamanı paketiyle yazılır: `fuse3` (FUSE için).

## 9. Açık kararlar

### Kullanıcının kararı

- **K0 — Google ve Microsoft kayıtlarının sahibi.** Kimin adına açılıyor (kişi mi, kurulacak
  bir kuruluş/alan adı mı)? Doğrulama ekranında ve izin ekranında bu ad görünür. Bugün iki
  kimlik de boş; Google ve Microsoft hiç sunulmuyor.
- **K1 — Gmail.** Seçenekler:
  - (a) Kendi istemcimizle kısıtlı kapsam `https://mail.google.com/` için inceleme başvurusu.
    Veri yerelde kaldığı için CASA beklenmiyor, ücretsiz. Gizlilik politikası, ana sayfa, alan
    adı doğrulaması ve demo videosu gerekiyor; süre haftalarla ölçülüyor **(doğrulanmadı)**.
    KDE benzer bir incelemede (Drive) reddedildi.
  - (b) Gmail'i sunmamak.
  - (c) Yalnızca uygulama parolasıyla IMAP. Hâlâ desteklendiği **(doğrulanmadı)**.

  Öneri: (a). İnceleme bitene kadar Gmail, 100 kullanıcı sınırı ve uyarı ekranıyla "deneysel"
  etiketiyle sunulur.
- **K2 — Google Drive.** `drive` kısıtlı ve dosyaları FUSE ile bağlayınca içerik yine cihazda
  kalıyor, yani CASA beklenmiyor. Ama Google, KDE'nin aynı amaçlı Drive erişimini engelledi.
  Öneri: Faz 4'te Nextcloud ve OneDrive önce; Drive başvurusu Gmail onayı alındıktan sonra,
  aynı gerekçe paketiyle. CASA ücreti (≈ 540–6 000 $/yıl) ancak Google sunucu tarafı veri
  saydığında gündeme gelir; o durumda karar yeniden kullanıcıya gelir.
- **K3 — Microsoft yayımcı doğrulaması.** Ücretsiz, ama Partner Program hesabı, iş hesabı ve
  kendi alan adı gerekiyor. Yoksa kurumsal kullanıcılar yönetici onayı görür; kişisel hesaplar
  etkilenmez.
- **K4 — GOA içe aktarma (Faz 5).** Yapılsın mı? Öneri: düşük öncelik, yalnız salt okunur.
- **K5 — iCloud Drive sunulmasın** (resmî yol yok). Öneri: sunulmaz, açıklama metni gösterilir.

### Mimari

- **K6 — Kodun yeri.** Öneri: ayrı `accounts` reposu (accountsd + `nuxut-accounts` +
  `nuxut-accounts-ipc`). Seçenek: calendar workspace'inde kalıp adı değişir. Hesaplar shell'e
  özgü değil; framework de UI çerçevesi olduğu için uygun değil.
- **K7 — Posta için G/Ç modeli.** Tokio yok kuralıyla IMAP IDLE: hesap başına bloklayan thread
  (senkron IMAP; `imap` crate'inin bakım durumu **doğrulanmadı**) ya da `io-imap` gibi G/Ç'siz
  bir protokol katmanı + kendi thread'imiz. Öneri: G/Ç'siz katman. Async çalışma zamanı
  getirmez, nuxut-net'in TLS ve güven kurallarıyla birleşir.
- **K8 — FUSE crate'i.** `fuser` (yaygın; bakım biçimi değişti, risk izlenmeli) ya da
  `polyfuse` (küçük, yalnız Linux). Faz 4 başında küçük bir deneme ile seçilir.
- **K9 — Tüketici yetkilendirmesi.** §3'teki "logla + hizmete göre kapsam" yeterli mi? Daha sıkısı:
  `GetAccessToken`'ı yalnız bilinen Nuxut ikililerine (`/proc/pid/exe` yolu) vermek. Ama yol
  sahteciliğine açık ve aynı kullanıcı kimliği altında gerçek bir sınır değil.
