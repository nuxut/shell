# polkit kimlik doğrulama ajanı (`nuxut-polkit`)

Onay: 2026-10-12 (güvenlik açısından kritik; iniş öncesi düşmanca
test + güvenlik incelemesi).

## Amaç

"Yönetici parolası gerekiyor" anında Nuxut görünümlü, tek kartlı bir diyalog. Bugün bu işi
kullanıcının Hypr zincirindeki `hyprpolkitagent` (Qt) yapıyor. Ayrıca timedated, UPower şarj
eşiği, udisks, accountsservice gibi sonraki işlerin önünü açar.

## Tehdit modeli

**Korunan:** kullanıcının (ya da seçilen yöneticinin) parolası ve polkit kararının bütünlüğü.
**Saldırgan:** aynı oturumda çalışan güvenilmeyen ama ayrı uid'li olmayan istemci değil — D-Bus
üzerinden ajanı kandırmaya çalışan herhangi bir bus istemcisi (sahte `BeginAuthentication`,
sahte mesaj/ikon ile oltalama), yardımcıdan gelen bozuk/uzun satırlar, diyaloğu çökertmeye
çalışan girdi.
**Kapsam dışı:** aynı uid'le çalışan kötü amaçlı kod (ajanı öldürüp kendi ajanını kaydedebilir;
her polkit ajanı için aynı). Root ve polkitd güvenilir.

**Temel özellikler (her biri bir test):**
1. *Yalnızca polkitd:* `BeginAuthentication` / `CancelAuthentication` çağrısının göndereni,
   sistem bus'ında `org.freedesktop.PolicyKit1` adının o anki sahibi (benzersiz ad) değilse
   `org.freedesktop.PolicyKit1.Error.Failed` ile reddedilir, diyalog açılmaz.
2. *Parola sızmaz:* parola yalnızca diyalog sürecinde; log, `Debug`, IPC, D-Bus, IME
   (surrounding text), komut satırı, ortam değişkeni yok. Diyalog süreci `PR_SET_DUMPABLE=0`
   (`nuxut_pam::process`). **Bellekteki tutucular (gerçek durum):** asıl parola tutucu
   framework `TextInput`'un `SecretBuffer`'ıdır — temizlemede/düşürmede sıfırlanır ama
   **mlock'lu değildir** (takasa düşebilir). Gönderimde `with_secret` ile mlock'lu, sıfırlanan
   `nuxut_pam::SecretBuf`'a kopyalanır ve yardımcıya yazılır; satır tamponu da sıfırlanır.
   Yardımcıya yazım arabelleksizdir (soket / çocuk stdin; `BufWriter` yok). PAM'in görünür
   (`ECHO_ON`) istemlerinde alan maskesiz kurulur (gizli sayılmaz); gizli istemde
   `TextInput::password(true)`.
   **Tuş/IME yolu (framework aa57e2a):** `KeyEvent::text` ve IME
   `Preedit`/`Commit` artık `SecretText` (Drop'ta sıfırlanır); parola alanı odaktayken preedit
   hiç tutulmaz, compositor'a `content_type` purpose=password + sensitive_data bildirilir,
   çevre metin gönderilmez. **Kalan sınırlar** (bizim elimizde değil): sctk / xkbcommon /
   wayland-backend iç kopyaları, compositor ve IME süreci, GPU'ya giden glif tamponu, takas
   (`SecretBuffer` mlock'suz) — ayrıntı framework `docs/ARCHITECTURE.md` "Gizli metin
   girişi".
3. *Protokol birebir:* yardımcıyla konuşma polkit'in kendi ajan kütüphanesiyle
   (`src/polkitagent/polkitagentsession.c`, polkit 127) aynı; bilinmeyen satır = başarısızlık,
   satır uzunluğu sınırlı (ör. 4 KiB), geçersiz UTF-8 kayıpsız reddedilir.
4. *Kendi oturumu:* ajan yalnızca kendi logind oturumuna (`unix-session`, `GetSessionByPID`)
   kaydolur; `fallback = false`.
5. *İptal ve zaman aşımı:* `CancelAuthentication` (çerezle) diyaloğu ve yardımcı bağlantısını
   kapatır, `BeginAuthentication` `org.freedesktop.PolicyKit1.Error.Cancelled` döner. Kullanıcı
   Esc/İptal → aynı. Diyalog etkileşimsiz `[polkit] timeout` (varsayılan 5 dk) sonra iptal.
6. *Fail-closed:* diyalog süreci çökerse/öldürülürse istek başarısız döner (yetki verilmez —
   karar zaten polkitd'de, yardımcı `SUCCESS` yazmadıkça yetki yok).
7. *Oltalama sınırı:* eylem açıklaması polkitd'den gelir, düz metin gösterilir (markup yok),
   uzunluk kırpılır, denetim/yön karakterleri atılır. Tanınması gereken metinler (isteyen yol,
   süreç adı, giriş adı, eylem kimliği) atılmaz, **kaçışlı** gösterilir (düz ASCII dışı her
   karakter `\u{…}`: homoglyph, boşluk, satır, yön) ve **ortadan** kırpılır (kök bileşen ve
   dosya adı görünür). İsteyen yalnızca doğrulanmış sistem yoluysa etiketsiz; değilse
   "doğrulanmamış konum/ad" (uygulama notları 3).
8. *Yazı-önü yok:* diyalog ilk kez sunulduktan sonra 400 ms boyunca tuşlar (Enter/Esc dahil)
   ve IME metni yok sayılır: `Exclusive` odak, kullanıcının başka pencerede yazdığının kalanını
   parola alanına alıp Enter'la göndermesin (pam_faillock `deny=3`). `InputGate`, saf ve
   saat enjeksiyonuyla testli.

## Süreç mimarisi

```text
nuxut-polkit            (yerleşik ajan; yalnızca zbus, Wayland/GL yok — boşta birkaç MB)
  └─ nuxut-polkit dialog  (istek başına; Wayland + framework; yardımcıyla tüm konuşma burada)
        └─ /run/polkit/agent-helper.socket  (polkit 127, soket etkinleştirme)
           yoksa /usr/lib/polkit-1/polkit-agent-helper-1 (setuid yolu, eski polkit)
```

- **Ajan (daemon):** sistem bus'ında `RegisterAuthenticationAgentWithOptions(subject, locale,
  object_path="/org/nuxut/PolicyKit1/AuthenticationAgent", {"fallback": false})`. Kayıt
  "already exists" ile reddedilirse (başka ajan, ör. hyprpolkitagent) açık uyarı loglar ve
  geri çekilmeli yeniden dener (30 sn, 2 dk, 5 dk…). polkitd adı el değiştirince
  (`NameOwnerChanged`): yeni sahip boşsa yalnızca bekleyen istekler iptal edilir; yeniden
  kayıt yalnızca yeni sahip boş değilse **ve** kaydı kabul eden sahipten (kayıt yanıtının
  göndereni; yoksa `GetNameOwner`) farklıysa. Neden: polkitd yeniden başlarken `eski→""` ve
  kayıt çağrısının bus etkinleştirmesiyle `""→yeni` gelir; ikincisi kayıt yanıtından önce
  tampona düşer ve eski mantıkta ikinci kayıt → "already exists" → sonsuz uyarı döngüsü
  doğuruyordu. İstekler sıraya girer; aynı anda tek diyalog.
- **Diyalog (çocuk süreç, aynı ikili `dialog` alt komutu):** stdin'den tek bir JSON istek
  (çerez, eylem kimliği, mesaj, ikon adı, kimlik listesi, ayrıntılar), stdout'a tek satır sonuç
  (`success` | `cancelled` | `failed`). Parolayı bu süreç alır ve yardımcıya yalnızca bu süreç
  yazar — parola ajan sürecinin adres alanına hiç girmez. Ajan kapanırsa çocuk
  `PR_SET_PDEATHSIG` / stdin EOF ile kapanır. İptal: ajan çocuğa SIGTERM, çocuk yardımcı
  bağlantısını kapatır.
  Gerekçe: boşta GL/font belleği yok (lock'taki gibi tek atımlık arayüz), çökme yalıtımı, parola
  kısa ömürlü süreçte.
- Kimlik seçimi: `unix-user` (uid → ad, `getpwuid` yerine `/etc/passwd` okuması ya da `uzers`
  crate'i — worker değerlendirir), `unix-group` (üyeler). Varsayılan: çağıran kullanıcı listede
  ise o, değilse ilk kimlik. Birden çoksa seçici.
- Başarısız deneme: yardımcı `FAILURE` → kart sallanır, "Parola yanlış", yeni yardımcı oturumu
  (polkit davranışı). PAM'in `PAM_ERROR_MSG` / `PAM_TEXT_INFO` metinleri gösterilir
  (parmak izi istemi böyle gelir).

## Arayüz

Framework layer-shell `Overlay`, `KeyboardMode::Exclusive`, odaklı çıkışta (compositor seçer); arka plan karartma
(tüm çıkışlarda ayrı karartma yüzeyi — v1 yalnızca odaklı çıkış). Kart: ikon, başlık ("Yetki
gerekiyor"), polkit mesajı, isteyen uygulama, kimlik (seçici), parola alanı (maskeli), hata
satırı, İptal / Doğrula. Bileşenler `nuxut-lock` ile aynı aileden (parola alanı); shell'e özgü
olmayan eksik bileşen (modal kart yüzeyi) → talep. Renk/hareket yalnızca tema
token'larından. Metinler i18n anahtarı (çeviri kataloğu).

## Yapılandırma

`shell.toml [polkit]`: `enabled = true`, `timeout_s = 300`. (Şema satırı kabukta.)

## Kurulum ve geçiş

- Çalışma alanı üyesi `polkit/` (onayla kök `Cargo.toml`), ikili `nuxut-polkit`,
  `unsafe_code = "forbid"`.
- systemd kullanıcı birimi `nuxut-polkit.service` (`polkit/dist/`, `PartOf=` ve
  `WantedBy=graphical-session.target`). install.sh ikiliyi ve birimi HER ZAMAN kurar;
  **etkinlik kullanıcının seçimidir** (uygulanan, 2026-10-12; `--polkit-switch` önerisi
  yerine): ilk kurulumda başka ajan yoksa `enable`, varsa etkinleştirmez ve geçiş yönergesi
  yazdırır; zaten etkinse korunur (`try-restart`), ikisi birden varsa uyarır. Başka ajan
  tespiti: bu kullanıcının süreçleri (`pgrep -u -f`, bilinen ajan adları: hyprpolkitagent,
  gnome/kde/mate/lxqt/xfce ajanları, soteria, polkit-dumb-agent, cosmic-osd,
  budgie-polkit-dialog), etkin `hyprpolkitagent.service`, Hyprland zincirinde yorumsuz başlatma satırı (Nuxut dosyaları hariç).
- Üretilen Hypr config (`nuxut.lua` / `nuxut.conf`, yalnızca kabuğu da başlattığı kipte):
  `systemctl --user is-enabled --quiet nuxut-polkit.service && systemctl --user
  import-environment WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE XDG_CURRENT_DESKTOP && systemctl
  --user start nuxut-polkit.service`; Lua'da oturum sonu `stop`. Doğrudan başlatma yedeği YOK
  (systemd'siz oturumda kullanıcının zinciri geçerli). İçe aktarma şart: diyalog Wayland'e
  ajanın (birimin) ortamından bağlanır.
- `hyprpolkitagent` ile çakışma: kullanıcının ajanına, servisine ve kendi config dosyalarına
  DOKUNULMAZ. İkisi de etkinse oturum başında polkitd'ye önce kaydolan kazanır; nuxut-polkit
  kaybederse geri çekilir ("already exists", 30 sn / 2 dk / 5 dk yeniden dener). Kaldırma
  birimi `disable --now` edip siler.

## Test planı

- **Birim:** yardımcı satır ayrıştırıcı (tüm satır tipleri, uzun satır, bozuk UTF-8, beklenmeyen
  satır), kimlik seçimi, istek kuyruğu + iptal durum makinesi, gönderen doğrulaması (saf işlev),
  diyalog JSON sözleşmesi (parola alanı YOK).
- **Bütünleşik (dbus-run-session, test ortamı kurulduktan sonra):** `fake-polkit` özelliği
  (yalnızca debug; sürümde `compile_error`, lock `fake-auth` kalıbı) ile ajan oturum bus'ında
  sahte Authority'ye kaydolur; sahte Authority `BeginAuthentication` çağırır. Sahte yardımcı
  (test ikilisi) protokolü konuşur, beklenen parola sabit. Senaryolar: başarı, yanlış parola →
  yeniden deneme, iptal (kullanıcı), `CancelAuthentication`, zaman aşımı, diyalog öldürülür,
  ajan öldürülür → çocuk kapanır, yabancı göndericiden `BeginAuthentication` reddi, kuyruk.
- **Özel bus gerekliliği:** `polkit/tests` özel bir `dbus-daemon` başlatır; yoksa test
  **başarısız olur** (sessizce geçen güvenlik testi yok). Bilinçli atlama yalnızca
  `NUXUT_POLKIT_SKIP_DBUS_TESTS=1` ile.
- **Canlı GUI:** yalnızca izole sway; sahte yardımcı; `wtype` yalnızca test display'ine; kendi
  yüzeyini kırparak ekran görüntüsü.
- **Gerçek polkitd:** parola GÖNDERİLMEZ (faillock kullanıcının hesabını kilitleyebilir).
  Kullanıcının oturumuna kayıt YOK (hyprpolkitagent kayıtlı; ona dokunulmaz). En fazla:
  `unix-process` öznesiyle (kendi test sürecimiz) kayıt + iptal yolu — onayla.

## Uygulama notları ve tasarımdan sapmalar (2026-10-12)

Kod: `polkit/` (ikili `nuxut-polkit`). Yardımcı protokolü polkit 127 kaynağından doğrulandı;
ayrıntı ve kaynak satırları `polkit/src/protocol.rs` modül belgesinde.

**Doğrulanan protokol (polkit 127):** `polkitagentsession.c` önce
`/run/polkit/agent-helper.socket`'e bağlanır (`<kullanıcı>\n<çerez>\n` yazar; yardımcı
`--socket-activated` ile istemciyi `SO_PEERPIDFD`+`SO_PEERCRED`'den tanır ve polkitd'ye
`AuthenticationAgentResponse2` gönderir), bağlanamazsa `/usr/lib/polkit-1/polkit-agent-helper-1
<kullanıcı>` başlatıp stdin'e `<çerez>\n` yazar. Yardımcı satırları `g_strescape` ile kaçışlı
`PAM_PROMPT_ECHO_OFF|PAM_PROMPT_ECHO_ON|PAM_ERROR_MSG|PAM_TEXT_INFO <metin>`, `SUCCESS`,
`FAILURE`; yanıt `<metin>\n` (yardımcı `fgets` ile ≤ 511 bayt okur). polkitd kimlik listesindeki
grupları kendisi kullanıcılara açar (`polkitbackendinteractiveauthority.c`), ajana yalnızca
`unix-user` gelir. polkitd `BeginAuthentication`'ı zaman aşımısız çağırır; yanıt hatası
`…Error.Cancelled` ise "kullanıcı vazgeçti", normal dönüşte karar `is_authenticated`'tır.

**Sapmalar (gerekçeli):**
1. *Oturum öznesi:* `GetSessionByPID` → `$XDG_SESSION_ID` (logind'de var ve bizim uid'imize ait)
   → kullanıcının `Display` oturumu. Neden: systemd kullanıcı birimi oturum kapsamında değildir
   (bu makinede de kullanıcı süreçleri için `GetSessionByPID` "does not belong to any known
   session" döndü). `fallback = false` korunur; polkitd ayrıca kullanıcı eşleşmesini denetler.
2. *Kimlik çözümü:* `uzers`/`/etc/passwd` yerine `nuxut_pam::lookup_uid` (`getpwuid_r`, NSS
   uyumlu, ağaçta). Yalnızca `unix-user` (yukarıdaki polkitd davranışı); çözülemeyen uid atlanır.
3. *İsteyen uygulama:* önce `polkit.subject-pid` (yetkiyi isteyen uygulama), yoksa
   `polkit.caller-pid` (çoğu zaman mekanizma servisi, ör. udisksd). Her PID için önce
   `/proc/<pid>/exe` yolu. **Doğrulanmış** (`Requester::Exe`, etiketsiz) yalnızca: geçerli
   UTF-8 ve yalnızca düz ASCII (`!`..=`~`), kanonik mutlak yol; ev dizini, `/tmp`, `/var/tmp`,
   `/run/user`, `/dev/shm` altında değil; çalışan inode (`/proc/<pid>/exe` üzerinden stat,
   yol üzerinden değil — yarışsız) normal dosya, sahibi root, grup/diğer yazamaz; yol hâlâ aynı
   inode'u gösteriyor (silinmiş/değiştirilmiş değil); tüm üst dizinler (sembolik bağlantı
   izlenmeden) root sahipli, grup/diğer yazamaz gerçek dizinler (yapışkan /tmp dahil
   reddedilir). Aksi hâlde `UnverifiedExe` → "doğrulanmamış konum"
   (`polkit-detail-requester-unverified-path`), ev dizini `~` ile. exe okunamazsa (başka
   kullanıcının süreci) `comm` — süreç `PR_SET_NAME` ile kendisi seçer — "doğrulanmamış ad"
   (`polkit-detail-requester-unverified`). Gösterim diyalogda: kaçış + ortadan kırpma
   (`text::display_path`): `~/.cache/x/usr/bin/gnome-software` asla `…/usr/bin/gnome-software`
   gibi görünmez. Sözleşmede `requester: {"exe"|"unverified_exe"|"comm": …}`, ham değer.
   Eylem kimliği ajanda ham (≤ 512 bayt), diyalogda kaçışlı + ortadan kırpılmış. Eylem mesajı
   ve PAM metinleri çevrilmiş metindir (kaçış yok, temizlik + sondan kırpma); not: pkexec
   mesajı kullanıcının verdiği program yolunu içerebilir — güvenilir bağlam isteyen satırı
   ve eylem kimliğidir.
4. *İkon:* polkit ikon adı tema dosyasından çözülmez; anlamsal kilit ikonu çizilir. Parolayı
   tutan süreçte görsel çözücü çalışmasın (saldırı yüzeyi).
5. *Protokolde polkit'ten katı:* satır ≤ 4 KiB, `SUCCESS`/`FAILURE` tam eşleşme (polkit önek
   eşler), NUL/bozuk UTF-8/sonda tek `\` reddedilir; kurallara uymayan satır = kesin `Failed`
   (yeniden deneme yok). Yanıt ≤ 510 bayt, `\n`/NUL içeremez.
6. *Yeniden deneme:* parola gönderildikten sonra `FAILURE`/EOF → hata satırı + sarsılma + yeni
   yardımcı oturumu (sınırsız; her biri kullanıcı girdisi ister, PAM faillock sınırlar). Parola
   sorulmadan art arda 2 başarısızlık → `Failed` (bozuk kurulumda döngü yok).
7. *Zaman aşımı:* etkileşimsizlik diyalogda (varsayılan 300 sn, `timeout_s` 30..=3600; tuş,
   tıklama, IME süreyi yeniler). Ajan ayrıca diyalog takılırsa mutlak sınır uygular
   (4 × zaman aşımı, en az 60 sn) → `Cancelled` + SIGTERM → 3 sn sonra SIGKILL → 3 sn sonra yuva
   zorla boşaltılır.
8. *Çıkış:* compositor seçer (`LayerSurfaceSpec::on_focused_output`, framework 3e1c952): odaklı/etkin
   çıkış; karartma yalnızca o çıkışta. Çıkış kaldırılınca yüzey kapanır → iptal (fail-closed).
9. *Sinyaller pidfd ile* (diyalog süreci): biçilmiş sürecin pid'i yeniden kullanılsa da yanlış
   sürece sinyal gitmez. setuid yardımcıya SIGTERM, biçme ile aynı kilit altında.
10. *Yapılandırma:* `[polkit]` tablosunu `nuxut-polkit` kendisi okur (salt okunur, bozuksa
    varsayılan + uyarı; `enabled = false` → kaydolmadan çıkış 0). Shell şema satırı kabukta.
11. *Diyalog sonucu:* stdout'ta tam olarak `success\n`/`cancelled\n` ve çıkış kodu 0 değilse
    `Failed` (çökme, öldürülme, fazladan çıktı → yetki yok). Loglar stderr'de.

**Test kancaları (`fake-polkit`, yalnızca debug; sürümde `compile_error`; özellik kapalıyken
ikilide adları bile yok — `tests/no_fake_polkit.rs`):** `NUXUT_POLKIT_FAKE_BUS` (sistem bus'ı
yerine özel bus), `NUXUT_POLKIT_FAKE_HELPER_SOCKET` (sahte yardımcı; setuid geri dönüşü kapanır),
`NUXUT_POLKIT_FAKE_HEADLESS` + `NUXUT_POLKIT_TEST_PASSWORDS` (başsız diyalog; sahte olmayan
yardımcı yolunda çalışmayı reddeder). Sahte yardımcı ikilisi `nuxut-polkit-fake-helper`.

## İsteyen süreç: TOCTOU bulgusu ve düzeltme (2026-10-13)

**Bulgu (güvenlik incelemesi, 216c467 sonrası):** isteyen etiketi `polkit.subject-pid`
/ `polkit.caller-pid` PID'inden `/proc` okunarak üretiliyordu. (a) *PID yeniden kullanımı:*
özne istekten sonra çıkar ve PID'i başka (kök sahipli sistem ikilisi çalıştıran) bir süreç
alırsa diyalog yanlış süreci "doğrulanmış" gösterebilirdi. (b) *Okuma sırasında exec:* `exe`
ve stat okumaları arasında süreç başka ikiliye geçebilirdi.

**polkitd ne veriyor:** `BeginAuthentication` `details` içinde yalnızca PID dizgileri var
(`polkitbackendinteractiveauthority.c` `add_pid`); öznenin başlangıç zamanı ya da pidfd'si ajana
iletilmez. Bu yüzden karşılaştırma polkitd'nin start-time'ıyla değil, **isteğin geliş anıyla**
yapılır.

**Düzeltme (onayla iner):** `request::read_proc(pid, arrival)`.
`arrival` = `BeginAuthentication` girişinde CLOCK_BOOTTIME (saat tıkı; `/proc/<pid>/stat`
22. alanıyla aynı birim ve saat). Sıra: `pidfd_open` → başlangıç zamanı → `exe` → `comm` → `exe`
→ başlangıç zamanı → pidfd hâlâ yaşıyor mu (`poll` POLLIN yok).
- Başlangıç zamanı `arrival`'dan sonra → o PID **atlanır** (asıl istekçi çıkmış; ne yol ne ad
  gösterilir), sonraki anahtara (`caller-pid`) geçilir.
- İki başlangıç zamanı farklı, iki `exe` okuması (yol + çalışan inode stat'ı) farklı, pidfd
  açılamadı ya da süreç okuma bitmeden çıktı → `stable = false` → yol sistem yolu olsa bile
  `UnverifiedExe` ("doğrulanmamış konum"). pidfd okuma boyunca yaşıyorsa PID arada yeniden
  kullanılamaz; başlangıç zamanı ≤ `arrival` o sürecin istekten önce var olduğunu gösterir.
- Etiket istek anına aittir: okuma bir kez, istek gelince yapılır; diyalog yeniden okumaz.
- Testler: `same_process` saf tablo (start-time uyuşmazlığı, istekten sonra başlama, exec, ölü
  pidfd), `requester` kararsız okumada `UnverifiedExe` ve atlanan özne → çağıran, gerçek /proc
  (istekten sonra başlatılan çocuk atlanır, önce başlamış olan tutarlı).

**Bilinen sınırlar:**
- *İstekten önce yapılmış exec ayırt edilemez:* süreç isteği yapıp (ya da polkitd'nin
  denetiminden sonra, isteğin bize ulaşmasından önce) kök sahipli bir sistem ikilisine `exec`
  ederse okuma tutarlıdır ve yol doğrulanmış görünür. Linux exec anını dışa vermez.
- *polkitd denetimi ile geliş anı arası:* özne bu kısa aralıkta çıkar ve PID'i yeni bir süreç
  alırsa yeni sürecin başlangıcı `arrival`'dan önce olabilir. Aralık D-Bus gecikmesi kadardır;
  polkitd pidfd/start-time iletmediği için kapatılamaz.
- Bu yüzden **isteyen satırı yalnızca bağlamdır**, güven kararı değil: ayrıntı satırında önce
  eylem (`Eylem: … · İsteyen: …`), küçük yazı (Caption) ve ikincil renkte, polkitd'nin mesajının
  altında. Asıl karar polkitd'nin eylem mesajı ve eylem kimliğidir.
