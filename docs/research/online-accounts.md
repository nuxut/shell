# Çevrimiçi hesaplar — araştırma (2026-10-16)

Hazırlayan: accounts-research (istek, 2026-10-16). Kaynaklar 2026-10-16'da açıldı;
sayfada güncelleme tarihi varsa parantez içinde verildi. Kaynakla doğrulanamayan her iddia
**(doğrulanmadı)** diye işaretli. Tasarım önerisi: `docs/design/accounts.md`.

Hedef (kullanıcı, 2026-10-16): "Hesap ekle" ile Google, Microsoft, Nextcloud, iCloud ve genel
IMAP/CalDAV eklenince takvim, kişiler, e-posta ve dosyalar masaüstüne tam bağlansın.

## Özet: en önemli bulgular

1. **GOA artık Google Drive vermiyor.** GOA 3.58.0 Google "Files" özelliğini kapattı, GNOME 50
   (Mart 2026) gvfs Google arka ucunu devre dışı bıraktı (libgdata 4 yıldır bakımsız ve
   arşivlendi; Google politikası nedeniyle değil). Debian gvfs-google'ı Ağustos 2025'te düşürdü.
   "Drive için GOA köprüsü" önermesi bugün geçersiz.
2. **Gmail için CASA, yalnızca yerelde veri tutan masaüstü uygulamasında gerekmiyor; ama kısıtlı
   kapsam incelemesi gerekiyor.** Google: güvenlik değerlendirmesi "kısıtlı kapsam verisini
   sunucularda saklıyor ya da iletiyorsanız" gerekli. IMAP/SMTP/POP (`https://mail.google.com/`)
   her zaman kısıtlı. Açık bir "yerel uygulama muafiyeti" cümlesi yok; muafiyet bu koşuldan
   çıkıyor.
3. **GNOME geliştiricileri GNOME'un OAuth anahtarlarının GNOME dışı projelerce kullanılmasını
   "sorunlu" buluyor** (jbicha, 2024-02-29). Google, KDE'nin Drive erişimini 2024-11'de engelledi,
   KDE 2025-02-17'de Drive iznini bıraktı. Başkasının anahtarına dayanmak üçüncü bir taraf
   kararına bağlanmak demek.
4. **GOA'ya hesap eklemenin D-Bus yolu yok.** Etkileşimli ekleme yalnızca `libgoa-backend`
   (GTK4 + libadwaita) ya da Mint'in `gnome-online-accounts-gtk` aracıyla yapılıyor. Okuma
   (`GetAccessToken`, posta/takvim/dosya özellikleri) GTK'sız, zbus ile mümkün.
5. **GOA'nın Microsoft 365 istemcisi doğrulanmamış** (GOA #335). Kurumsal kiracılarda "yönetici
   onayı gerekiyor" çıkıyor, kişisel Microsoft (Windows Live) sağlayıcısı 3.55.0'da kaldırıldı.
   Kendi Entra kaydımız (calendar ADR-0007) zaten daha iyi.
6. **iCloud Drive'ın resmî üçüncü taraf API'si yok.** Posta, takvim ve kişiler resmî ve kararlı
   (uygulamaya özel parola). Drive yalnızca tersine mühendislikle erişilebiliyor (rclone
   `iclouddrive`, Tier 4, gerçek Apple ID parolası, ADP kapalı olmalı).
7. **Nextcloud en temiz sağlayıcı.** Login Flow v2 + uygulama parolası; WebDAV, CalDAV, CardDAV
   ve bildirimler tamamen açık standart. Mevcut kodda Login Flow zaten var.

## 1. Google: izin sınıfları ve doğrulama

| Kapsam | Sınıf | Kaynak |
|---|---|---|
| `https://mail.google.com/` (IMAP/SMTP/POP) | kısıtlı | [Gmail scopes](https://developers.google.com/workspace/gmail/api/auth/scopes) (2026-09-10); [kısıtlı liste](https://support.google.com/cloud/answer/13464325): "includes any usage of IMAP, SMTP, and POP3" |
| `gmail.readonly`, `gmail.modify`, `gmail.metadata`, `gmail.compose` | kısıtlı | aynı |
| `gmail.send` | hassas | aynı |
| `gmail.labels` | hassas değil | aynı |
| `drive`, `drive.readonly`, `drive.metadata(.readonly)` | kısıtlı | [Drive auth](https://developers.google.com/workspace/drive/api/guides/api-specific-auth) (2026-09-03) |
| `drive.file`, `drive.appdata` | hassas değil | aynı. `drive.file` yalnızca kullanıcının uygulamayla seçtiği ya da açtığı dosyalara erişir; Drive'a göz atma yok |
| `calendar*`, `contacts*` | büyük olasılıkla hassas | Google'ın hassas kapsam sayfası "Google Takvim'deki etkinlikleri okumak" ve "Kişiler'e kişi kaydetmek" örneklerini veriyor ([sensitive-scope-verification](https://developers.google.com/identity/protocols/oauth2/production-readiness/sensitive-scope-verification), 2026-08-19). Kısıtlı listede yoklar. Kapsam başına etiket **(doğrulanmadı)**: Cloud Console "Data Access" sayfasında görülür |

**Kısıtlı kapsam doğrulaması:**
- Kaynak: [restricted-scope-verification](https://developers.google.com/identity/protocols/oauth2/production-readiness/restricted-scope-verification) (2026-08-19).
- Güvenlik değerlendirmesi (CASA): "If you store or transmit restricted scope data on servers,
  then you need to complete a security assessment". Ayrıca "from or through a third-party
  server" erişen uygulamalar.
  - Verisi yalnızca cihazda kalan masaüstü uygulaması bu koşula girmiyor, ama kısıtlı kapsam
    incelemesinden yine geçiyor: politika uyumu, gerekçe, demo videosu.
  - Açık kaynak hiçbir sayfada ayrıca anılmıyor.
- Yıllık döngü: CASA geçen uygulamalar "en az 12 ayda bir" yeniden değerlendiriliyor. Seviyeler
  AL1/AL2, risk ve kullanıcı sayısına göre ([13465431](https://support.google.com/cloud/answer/13465431)).
- CASA ücretini geliştirici öder. Fiyatlar (değerlendiriciye göre değişir; bütçe için teyit
  edilmeli):
  - TAC Security Tier 2 ≈ 540–720 $, Tier 3 ≈ 3 600 $ ([TAC](https://tacsecurity.com/esof/)).
  - Leviathan Tier 2: 3 000–6 000 $ ([Leviathan](https://leviathansecurity.com/programs/google-casa-cloud-application-security-assessment)).
  - Kasım 2024 karşılaştırması: TAC Tier 3 ≈ 4 500 $ ([switchlabs](https://www.switchlabs.dev/post/casa-tier-2-tier-3-security-review-providers-pricing-and-the-cheapest-option)).
- Doğrulanmamış ya da "Testing" durumundaki uygulamalar:
  - Uyarı ekranı ve 100 kullanıcı sınırı; sınır "yalnızca doğrulama başarıyla bitince
    kalkar" ([13464323](https://support.google.com/cloud/answer/13464323)).
  - "Testing" durumunda yenileme belirteci 7 günde düşer (yalnızca
    openid/email/profile istenmiyorsa) ([oauth2](https://developers.google.com/identity/protocols/oauth2)).
- Masaüstü istemcisi ([native-app](https://developers.google.com/identity/protocols/oauth2/native-app), 2026-10-15):
  - `client_secret` gizli tutulamaz.
  - Yönlendirme loopback `http://127.0.0.1:port`; OOB ve özel URI şeması desteklenmiyor.
  - PKCE S256 öneriliyor.

**Mevcut karar** (calendar `docs/google-verification/`, ADR-0007, 2026-10-11):
- Kullanıcı Nuxut'un kendi Google Cloud projesini "Desktop app" istemcisiyle açıyor.
- Kapsamlar `openid`, `userinfo.email`, `calendar.calendarlist.readonly`, `calendar.events`.
  Geniş `calendar` bilerek istenmiyor.
- "Yayında, doğrulanmamış" durumda (7 gün sınırı yok); hassas kapsam doğrulaması ücretsiz.
- Kimlik `crates/nuxut-accounts/src/client_ids.rs` içinde. Şu an **boş**, Google bu derlemede
  kapalı.

## 2. GNOME Online Accounts (GOA)

**Sürüm ve bağımlılıklar** (`pacman -Si/-Qi`, 2026-10-16):
- `gnome-online-accounts` 3.58.1-1 bu VM'de kurulu, 1.8 MiB.
- Bağımlılıklar: gtk4, libadwaita, gcr-4, librest, libsoup3, libsecret, krb5.
- **WebKit bağımlılığı yok.** NEWS 3.49.0: "Remove the webview and WebKit dependency". OAuth sistem
  tarayıcısında; `goa-oauth2-handler` `x-scheme-handler/goa-oauth2` şemasını alıyor.
- `libgoa` (istemci kütüphanesi) yalnızca glib2'ye bağlı.
- `org.gnome.OnlineAccounts` oturum bus'ında etkinleştirilebilir, şu an çalışmıyor
  (`busctl --user list`).

**NEWS'ten** ([NEWS](https://gitlab.gnome.org/GNOME/gnome-online-accounts/-/raw/master/NEWS)):
- 3.49.1: GTK4/Adwaita + Microsoft 365 sağlayıcısı.
- 3.50.1: PKCE.
- 3.51.1: M365 için Mail/Calendar/Contacts.
- 3.54.0: Google client secret kaldırıldı.
- 3.55.0: Windows Live kaldırıldı.
- 3.57.1: "#335 Default Microsoft 365 client is unverified".
- 3.58.0: Google Files kapatıldı.

**D-Bus API** (yerel `goa-1.0.vapi`/`Goa-1.0.gir`; XML dosyası açılmadı):
- `Account`: Id, ProviderType, Identity, PresentationIdentity, AttentionNeeded, `*Disabled`
  bayrakları (Mail, Calendar, Contacts, Files…). Yöntemler `EnsureCredentials()`, `Remove()`.
- `OAuth2Based.GetAccessToken() → (token, expires_in)`. `PasswordBased.GetPassword(id)`.
- `Mail`: EmailAddress, Imap*/Smtp* (Host, UserName, UseSsl/Tls, SmtpAuthXoauth2…).
- `Calendar`/`Contacts`/`Files`: `Uri`.
- `Manager.AddAccount(...)` önceden alınmış kimlik bilgisini kaydeder. **Etkileşimli akış
  başlatmaz.**

**Hesap ekleme:**
- Etkileşimli yol `goa_provider_add_account(provider, client, GtkWidget *parent, …)`
  (`goabackend/goaprovider.h`). `goa-backend-1.0.pc` gtk4 ve libadwaita-1 istiyor.
- gnome-control-center şart değil.
- Mint'in [gnome-online-accounts-gtk](https://github.com/linuxmint/gnome-online-accounts-gtk)
  aracı (Arch 3.50.11, gtk4 + libadwaita + xapp-symbolic-icons) bunu masaüstünden bağımsız
  yapıyor. GOA 3.58 ile tam uyumu **(doğrulanmadı)**; Arch birlikte paketliyor.

**Google sağlayıcısı** (`goagoogleprovider.c`, master):
- İstenen kapsamlar: userinfo.email/profile, tam `calendar`, `m8/feeds`, `carddav`,
  `https://mail.google.com/`, `tasks`.
- `drive` yalnızca `google_files` derleme seçeneğiyle (varsayılan kapalı).
- Açılan hizmetler: Mail, Calendar, Contacts (Files yok, Photos yok).
- GNOME'un istemci kimliği `meson_options.txt` içinde; dağıtımlar değiştirebilir.
- GNOME istemcisinin kısıtlı kapsam doğrulaması hakkında resmî açıklama bulunamadı
  **(doğrulanmadı)**.

**GNOME dışı kullanım:**
- GOA 3.49/3.50 (GTK4) Budgie, Cinnamon ve Unity panellerini kırdı. Budgie paneli kaldırdı.
- jbicha: "problematic for non-GNOME projects to be using GNOME Online Accounts directly anyway
  because of the limited use OAuth keys owned by the GNOME Foundation"
  ([Ubuntu Discourse](https://discourse.ubuntu.com/t/gnome-online-accounts-no-longer-compatible-with-budgie-cinnamon-unity-control-center/42898), 2024-02-29).
- Cinnamon, Xfce ve MATE artık Mint'in aracını kullanıyor
  ([Arch cinnamon #2](https://gitlab.archlinux.org/archlinux/packaging/packages/cinnamon/-/issues/2)).
- Google ya da GNOME tarafında yazılı bir yasak metni bulunamadı. BLFS dağıtıcılara "kendi
  anahtarlarınızı alın" diyor ([BLFS](https://linuxfromscratch.org/blfs/view/cvs/gnome/gnome-online-accounts.html)).

**Engelleme geçmişi:**
- 2019: GOA'da "This app has not been verified" ([LP#1847597](https://bugs.launchpad.net/bugs/1847597)).
- KDE KIO GDrive 2024-11'den itibaren "This app is blocked". KDE 2025-02-17'de Drive iznini
  bıraktı: "weren't able justify our API usage to their satisfaction"
  ([KDE discuss](https://discuss.kde.org/t/google-blocks-access-from-dolphin-kio-gdrive/26007)).

**Google Drive (GNOME 50):**
- Files anahtarı kaldırıldı, gvfs MR !384 Google arka ucunu kapattı. Gerekçe bakım: libgdata
  arşivlendi ve hâlâ libsoup2 kullanıyor
  ([GNOME discourse](https://discourse.gnome.org/t/google-drive-in-gnome-50/34417), 2026-03/04;
  [OMG Ubuntu](https://www.omgubuntu.co.uk/2026/03/google-drive-not-working-nautilus-ubuntu-26-04), 2026-03-31).
- Nisan 2026'da önerilen libsoup3 REST arka ucunun birleşip birleşmediği **(doğrulanmadı)**.

**OneDrive:**
- gvfs-onedrive (libmsgraph) GNOME 46'dan beri var; Arch 1.60.2.
- GNOME 48'de 403 ve "Database is read only" hata raporları var ([discourse](https://discourse.gnome.org/t/database-read-only-when-trying-to-open-onedrive-in-nautilus/38213)).

**Microsoft 365 sağlayıcısı:**
- Şubat 2025'te bildirilen varsayılan kapsamlar: files.readwrite(.all), sites.*, mail.readwrite,
  contacts.readwrite, offline_access. Yöneticiler kapsamları fazla geniş buluyor (#407)
  ([discourse](https://discourse.gnome.org/t/microsoft-graph-permissions-required-by-a-microsoft-365-account/26697)).
- `Calendars.*` listede yoktu **(doğrulanmadı)**.
- Varsayılan istemci doğrulanmamış ([#335](https://gitlab.gnome.org/GNOME/gnome-online-accounts/-/issues/335)).
  Kurumlar kullanıcılara kendi istemci ve kiracı kimliklerini veriyor
  ([UWaterloo](https://uwaterloo.atlassian.net/wiki/spaces/ISTKB/pages/44816171143/Gnome+Online+Accounts)).

## 3. Microsoft

**IMAP/SMTP OAuth** ([MS Learn](https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth)):
- Kapsamlar `https://outlook.office.com/IMAP.AccessAsUser.All`, `SMTP.Send`, `offline_access`.
- SASL XOAUTH2. Microsoft 365 ve Outlook.com hesaplarının ikisinde de çalışıyor.

**Graph:**
- `common` uç noktası kişisel ve iş/okul hesaplarını kapsıyor.
- Yetkilendirilmiş kapsamlar: `Calendars.ReadWrite`, `Contacts.ReadWrite`, `Mail.ReadWrite`,
  `Mail.Send`, `Files.ReadWrite`, `offline_access`.
- Public client + PKCE + `http://localhost` loopback. Bu oturumda yeniden açılmadı
  **(doğrulanmadı)**; mevcut takvim kodu bu yolla çalışıyor.

**Yayımcı doğrulaması** ([MS Learn](https://learn.microsoft.com/en-us/entra/identity-platform/publisher-verification-overview)):
- Ücretsiz: "Microsoft doesn't charge developers… No license is required."
- Şartlar:
  - Doğrulanmış Microsoft Partner Program (MCPP) hesabı (Partner One ID).
  - Uygulama iş/okul hesabıyla kayıtlı olmalı.
  - Yayımcı alan adı `*.onmicrosoft.com` olmamalı.
  - Hem Uygulama Yöneticisi hem Partner Center yöneticisi olan kişi, MFA ile.
- Doğrulanmamış çok kiracılı uygulamada, risk tabanlı onay açık kiracılar temel profil dışındaki
  izinleri kullanıcıya verdirmez; yönetici onayı gerekir. Kişisel hesaplar bundan etkilenmiyor
  (çıkarım).
- Graph yetkilendirilmiş kapsamları için inceleme ücreti bulunamadı **(doğrulanmadı)**.

**Mevcut karar** (calendar `docs/microsoft-registration.md`, ADR-0007):
- Entra kaydı `AzureADandPersonalMicrosoftAccount`, public client, `http://localhost`, cihaz
  kodu yedeği, sır yok, PKCE.
- Kapsamlar openid, email, profile, offline_access, `Calendars.ReadWrite`.
- Doğrulanmadan önce kurumsal hesaplar için `adminconsent` bağlantısı gösteriliyor.
- Kimlik şu an **boş**.

## 4. iCloud

- **Posta** ([Apple HT202304](https://support.apple.com/en-ie/HT202304)):
  - IMAP `imap.mail.me.com:993` (SSL); SMTP `smtp.mail.me.com:587` (STARTTLS, kimlik zorunlu,
    kullanıcı adı tam adres). POP yok.
  - Parola uygulamaya özel parola. Bunun için iki adımlı doğrulama gerekiyor (üçüncü taraf
    rehber: [mailmeteor](https://mailmeteor.com/smtp/icloud-smtp-settings)).
- **Takvim ve kişiler:** CalDAV `caldav.icloud.com`, CardDAV `contacts.icloud.com`, uygulamaya
  özel parolayla. Yaygın biliniyor, mevcut takvim `Caldav` sağlayıcısı bunu hedefliyor; Apple
  kaynağıyla bu oturumda doğrulanmadı **(doğrulanmadı)**.
- **iCloud Drive: resmî yol yok.**
  - CloudKit Web Services yalnızca bir uygulamanın kendi `iCloud.*` kapsayıcısına erişiyor
    ([Apple](https://developer.apple.com/library/content/documentation/DataManagement/Conceptual/CloudKitWebServicesReference/SettingUpWebServices.html)).
  - [rclone iclouddrive](https://rclone.org/iclouddrive/) v1.69'da geldi ve "Tier 4":
    - Gerçek Apple ID parolası + 2FA istiyor; uygulamaya özel parola kabul edilmiyor.
    - ADP kapalı ve "Access iCloud Data on the Web" açık olmalı.
    - Güven belirteci 30 günde bir yenilenmeli.
    - Sahada 423 "Missing PCS cookies" hataları var ([forum](https://forum.rclone.org/t/rclone-1-69-icloud-issue-http-error-423-missing-pcs-cookies-from-the-request/49494)).
  - Sonuç: ürün özelliği olarak sunulmamalı.

## 5. Nextcloud

- [Login Flow v2](https://docs.nextcloud.com/server/latest/developer_manual/client_apis/LoginFlow):
  - `POST /index.php/login/v2` → tarayıcı adresi + `poll{token,endpoint}`.
  - Yoklama onaya kadar 404, sonra **bir kez** 200 `{server, loginName, appPassword}`.
  - Belirteç 20 dakika geçerli.
  - Mevcut kod: `nuxut-accounts::nextcloud::LoginFlow`.
- Uç noktalar: WebDAV `remote.php/dav/files/USER`, `/.well-known/caldav|carddav` → `remote.php/dav`
  (standart belge; bu oturumda açılmadı **(doğrulanmadı)**).
- Yetenekler: `ocs/v2.php/cloud/capabilities`, notify_push websocket adresini de veriyor.
- Bildirimler: OCS `apps/notifications/api/v1/notifications` (yoklama) ya da notify_push
  (`notify_notification` olayı).
- Masaüstü istemcisinde Linux sanal dosyaları hâlâ deneysel (`showExperimentalOptions`)
  ([forum](https://help.nextcloud.com/t/linux-desktop-client-doesnt-support-virtual-files/244807)).

## 6. Dosyaları masaüstüne bağlama

| Yol | Ne | Artı | Eksi |
|---|---|---|---|
| Kendi FUSE (Rust [`fuser`](https://docs.rs/crate/fuser/latest) 0.18 ya da [`polyfuse`](https://lib.rs/crates/polyfuse) 0.5.1) | İsteğe bağlı bağlama + yerel önbellek | Her uygulama görür; Nuxut güvenlik kapısı (nuxut-net) ve belirteçleri tek yerde; kendi önbellek politikası | Yazması en büyük iş (çakışma, yükleme kuyruğu, çevrimdışı). `fuser` bakım biçimini değiştirdi (README); bakım riski izlenmeli |
| gvfs (gvfs-goa, gvfs-onedrive) | GIO sanal dosya sistemi | Hazır, GTK uygulamaları görür | GOA'ya bağlı; Google arka ucu kaldırıldı; OneDrive'da hata raporları; GTK dışı uygulamalar yalnızca `~/.gvfs` FUSE köprüsüyle görür |
| rclone mount (Go, dış ikili) | FUSE + VFS önbelleği | Çok sağlayıcı, olgun | Ayrı dil ve süreç. `full` önbellek kipi seyrek dosya ister, büyük dosya açarken bazen tümünü indiriyor ([forum](https://forum.rclone.org/t/writes-vs-full/22568)); belirteçler rclone yapılandırmasında |
| OneDriver (Go, FUSE) | OneDrive isteğe bağlı | v0.15.0 (2026-01), çevrimdışıyken salt okunur | Tek sağlayıcı, dış ikili |
| abraunegg/onedrive (D) | Tam eşitleme | Çevrimdışı tam | Disk tüketir; 2.5 sürümü 2.4 veritabanıyla uyumsuz |
| Nextcloud desktop (Qt) | Tam eşitleme | Resmî | Linux sanal dosyaları deneysel |

İsteğe bağlı bağlama ile tam eşitleme arasındaki fark (sentez):
- İsteğe bağlı: disk tasarrufu ve anında görünürlük. Ama her `stat`/`readdir` ağ gecikmesi
  taşıyor, önizleme üretimi dosyayı indiriyor.
- Tam eşitleme: hızlı ve çevrimdışı çalışıyor. Ama disk tüketiyor ve çakışma çözümü gerekiyor.
- İyi tasarım: varsayılan isteğe bağlı, klasör başına "çevrimdışı tut", uzak dosyada önizleme
  yalnızca önbellekteyse.

## 7. E-posta

- **XOAUTH2/OAUTHBEARER:** Gmail ve Outlook IMAP IDLE ile çalışıyor.
- **Rust crate'leri:**
  - [`async-imap`](https://docs.rs/async-imap): takılabilir `Authenticator`; IDLE özelliği
    **(doğrulanmadı)**. Async çalışma zamanı ister; Nuxut tokio kullanmıyor, calendar ADR-0004.
  - [`io-imap`](https://docs.rs/crate/io-imap/0.5.0): XOAUTH2, OAUTHBEARER ve SCRAM yerleşik.
  - `lettre`: `Mechanism::Xoauth2` ([Proxmox 2026](https://lore.proxmox.com/all/DHA5SHDM6BCI.XPD3SWDT8H97@proxmox.com/t)).
  - `mail-parser` (Stalwart) **(doğrulanmadı)**.
- **Thunderbird:**
  - Sağlayıcıları ve istemci anahtarlarını `OAuth2Providers.sys.mjs` içinde sabit tutuyor
    ([wiki](https://wiki.mozilla.org/Special:MobileDiff/1251178)). TB 153+ OAuth'u sistem
    tarayıcısında yapıyor ([SUMO](https://support.mozilla.org/ja/kb/tb-oauth)).
  - Hesabı Thunderbird'e devretmek mümkün değil: belirteç TB'nin kendi istemcisine bağlı. En
    fazla adres ve sunucu ön doldurulabilir **(doğrulanmadı)**.
- **Evolution:** evolution-data-server'ın GOA modülü GOA hesaplarından kaynak üretiyor
  **(doğrulanmadı, sayfa açılmadı)**. Yani Evolution Nuxut hesaplarını ancak Nuxut kendi EDS
  kaynağı yazarsa ya da hesaplar GOA'da olursa görür.
- **Gmail uygulama parolası** (2SV açık kişisel hesaplarda IMAP için) hâlâ var mı: **(doğrulanmadı)**.
  Bu oturumda kaynak açılmadı.

## 8. Güvenlik

- **Belirteç saklama:**
  - Secret Service, Rust [`oo7`](https://docs.rs/crate/oo7/latest) 0.6.0: D-Bus + dosya arka
    ucu; `oo7-portal`. Mevcut kod ADR-0005: yalnızca Secret Service, dosya yedeği bilerek yok.
  - `secret-service` crate'i değerlendirilmedi **(doğrulanmadı)**.
- **Yerel uygulama OAuth'u:**
  - Kurallar: RFC 8252 (loopback), RFC 7636 (PKCE S256), [RFC 9700](https://www.rfc-editor.org/rfc/rfc9700)
    (OAuth güvenlik BCP, Ocak 2025).
  - RFC 9700'ün implicit/ROPC'yi kaldırdığı ve public istemcide PKCE'yi zorunlu kıldığı
    biliniyor; bölüm bölüm okunmadı **(doğrulanmadı)**.
  - Mevcut `nuxut-accounts::oauth` bunları zaten uyguluyor: 127.0.0.1/::1, tek istek, 8 KiB,
    sabit zamanlı `state`.
- **`oauth2` crate'i** 5.0.0: kendi HTTP istemcimiz (nuxut-net, yönlendirme izlemiyor) zaten
  SSRF kurallarını uyguluyor. Geçiş gerekmez.
- **Tehdit modeli notu:**
  - Aynı kullanıcı kimliğiyle çalışan kötü amaçlı süreç Secret Service'i de okuyabilir. Asıl
    kazanç belirtecin **tek süreçte** tutulması, logsuzluk ve en az kapsam.
  - GOA'da da her oturum süreci `GetAccessToken` çağırabiliyor.
  - Ayrıştırıcıların (MIME, iCalendar, vCard, WebDAV XML) yalıtılması framework ADR-0007
    (görsel işçi: seccomp + landlock + rlimit) deseniyle yapılabilir.

## 9. Rakipler

| | Güçlü yan | Zayıf yan |
|---|---|---|
| **GNOME Online Accounts** | Tek yer, hizmet başına aç/kapa; D-Bus API; WebKit'siz tarayıcı OAuth'u; Evolution, Calendar ve Nautilus tüketiyor | Arka uç kütüphanesi bakımsız kalınca özellik sessizce düşüyor (Google Drive, GNOME 50); M365 istemcisi doğrulanmamış; Windows Live kaldırıldı; GTK4/libadwaita'ya bağlı; GNOME dışı masaüstlerini kırdı (3.50) |
| **KDE (KAccounts/signond)** | Paketler hâlâ çıkıyor (kaccounts-providers 25.08–26.04) | Plasma 6'da modül eksik paketle görünmüyor, Google girişi bozuk raporları ([discuss](https://discuss.kde.org/t/online-accounts-section-missing-in-system-settings-kubuntu-24-10/27468)); Google Drive engellendi |
| **Linux Mint / Cinnamon** | GOA arka ucu + kendi önyüzü (gnome-online-accounts-gtk) | Hizmetler GOA kadar; boş panel raporları ([EndeavourOS](https://forum.endeavouros.com/t/online-accounts-empty-no-options/54662?page=2)) |
| **macOS İnternet Hesapları** | Tek yer, hizmet başına anahtar; Mail, Takvim, Kişiler, Notlar ortak kullanıyor **(doğrulanmadı, Apple kaynağı açılmadı)** | Kapalı ekosistem |
| **Windows** | — | Mail/Takvim/Kişiler 31.12.2024'te kapatıldı, yerine new Outlook geldi ([MS](https://support.microsoft.com/topic/773ecb94-5b16-4155-96e1-bc9afdc08e31)); sistem hesabı tek uygulamaya kaydı |

**"Linux'ta en sağlam" iddiası için şart olanlar** (sentez):
1. Kendi doğrulanmış OAuth istemcilerimiz. Başkasının anahtarı başkasının kararına bağlı (KDE ve
   GNOME örnekleri).
2. Belirteçlerin tek sahibi olan bir hesap servisi ve kararlı D-Bus sözleşmesi.
3. Hizmet başına aç/kapa ve hesap başına görünür sağlık durumu: "yeniden bağlan", yönetici onayı,
   kota.
4. Bağımlılıkta "bakımsız kalan kütüphane" riskinin olmaması (gvfs-google dersi).
5. Bir hizmet sağlanamıyorsa dürüst metin (iCloud Drive, Google Drive kısıtı), sessizce bozuk
   seçenek değil. Mevcut kod kimliği olmayan sağlayıcıyı zaten gizliyor.
6. Çevrimdışı çalışma ve veri yalnızca cihazda. Bu aynı zamanda Gmail'de CASA'yı gereksiz kılan
   koşul.
