# Kilit ekranı tetikleyicisi (kabuk → `nuxut-lock`)

2026-10-16, karar (kullanıcı hatası: "Ekranı kilitle yine çalışmıyor").
Kod: `src/services/lock_launcher.rs`, `src/services/idle/logind.rs`
(`LogindEvent::LockRequested`), `src/app/session.rs`.

## Kök neden

Kilitle düğmesi logind'e `Session.Lock()` diyordu. logind bunun üzerine hiçbir şey kilitlemez;
yalnızca oturum nesnesinden `org.freedesktop.login1.Session.Lock` sinyalini yayınlar. Sinyali
dinleyip kilit ekranını açacak parça Nuxut'ta yoktu (ARCHITECTURE "Tetikleme": shell
tetikleyicisi kapalıydı); kullanıcıda hypridle yok → düğme sessizce hiçbir şey yapmıyordu.
Boşta kilidi (`[idle] lock`) ve uyku öncesi kilit de aynı `Session.Lock()`'a dayandığından onlar
da kilit ekranı açmıyordu. (`7f7e6e9` "oturum bulunamadı" düzeltmesi ayrı bir hataydı: çağrı
artık logind'e ulaşıyordu ama dinleyen yoktu.)

## Tasarım

Tek yol: **istek → logind `Session.Lock()` → `Lock` sinyali → kabuk `nuxut-lock`'u başlatır.**

| Kaynak | İstek |
| --- | --- |
| Oturum kartı Kilitle | `SessionService` → `Session.Lock()` |
| `nuxut-shell msg lock` | aynı (`Command::Lock` → `session_action(Lock)`) |
| SUPER+L (temel katman) | `<kabuk> msg lock \|\| loginctl lock-session` (ile anlaşma: kabuk çalışıyorsa onun yolu, fail-safe dahil; kabuk yoksa logind'e doğrudan, hypridle vb. dinler) |
| Boşta / uyku öncesi kilit | `IdleLogind` → `Session.Lock()` |
| Dış araçlar | `loginctl lock-session`, hypridle `before_sleep_cmd` |

- **Dinleyici:** `IdleLogind` (zaten tek logind bağlantısı; `PrepareForSleep`, `LockedHint`)
  oturumun `Lock` sinyalini `LogindEvent::LockRequested` olarak iletir. Boşta ayarlarından
  bağımsızdır: bağlantı her zaman açılır (test örneği hariç, aşağıda).
- **Sahtecilik:** sinyal yalnızca `org.freedesktop.login1`'in güncel benzersiz sahibinden
  (`NameOwner::accepts`, eşleme kuralı da `sender=org.freedesktop.login1`) ve yalnızca kendi
  oturum yolumuzdan (`GetSessionByPID` / `session/auto` ile çözülen) kabul edilir.
  `loginctl lock-sessions` her oturuma ayrı sinyal yollar; başka oturumunki yok sayılır.
- **`Unlock` iletilmez.** Kilidi yalnızca `nuxut-lock` içindeki kimlik doğrulama açar;
  `loginctl unlock-session` (aynı uid'den herhangi bir süreç gönderebilir) kilidi açmamalı.
- **Başlatma:** `LockLauncher` — kabuk ikilisinin yanındaki `nuxut-lock` (`current_exe`
  kardeşi; PATH'e güvenilmez), sabit argv (argümansız), kabuksuz, stdin `/dev/null`. Çocuk ayrı
  thread'de beklenir (zombi yok), çıkış kodu loglanır.
- **Çift kilit yok:** kendi çocuğumuz çalışırken ikinci tetik başlatmaz (`AlreadyRunning`).
  Başkası (hypridle `lock_cmd = nuxut-lock`) başlattıysa `nuxut-lock`'un tek örnek IPC'si
  ikinciyi hemen 0 ile çıkarır. Kullanıcının kendi farklı kilitleyicisi varsa (hyprlock,
  swaylock) compositor ikinci ext-session-lock'u reddeder: `nuxut-lock` 1 ile çıkar, kabuk bunu
  "başka bir kilitleyici etkin olabilir" diye loglar. Kalıcı çözüm ayar: `[lock] listen_logind =
  false` (varsayılan `true`: Nuxut dinler). Süreç taraması gibi kırılgan bir otomatik tespit
  yapılmadı; ret zararsız ve görünür.
- **Fail-safe (kilitlenemeyen kilit düğmesi yok):** `decide()` (saf, birim testli):
  - logind sinyali → `listen_logind` ise başlat;
  - `Session.Lock()` başarılı ama dinleyici hazır değil (logind bağlantısı yok ya da oturum
    çözülemedi: sinyal bize ulaşmaz) → `listen_logind` ise doğrudan başlat;
  - `Session.Lock()` başarısız / oturum servisi yok / logind yok → **her zaman** doğrudan
    başlat (dinleyici kapalı olsa da); kart hatayı yine gösterir.
  Boşta kilidi de aynı: `IdleLogind` yok ya da dinleyici hazır değilse doğrudan başlatılır,
  `Failed { action: Lock }` gelirse de.
- **Test örneği** (`NUXUT_SHELL_INSTANCE`): oturum servisi ve boşta logind bağlantısı gerçek
  sistem bus'ına bağlanmaz (yalnızca `DBUS_SYSTEM_BUS_ADDRESS` sahte bir bus'a yönlendirilmişse).
  Eskiden oturum servisi test örneğinde de gerçek logind'e `Lock`/`Terminate` gönderebiliyordu;
  artık göndermez. Doğrudan başlatma da test örneğinde yalnızca `NUXUT_LOCK_INSTANCE`
  tanımlıysa yapılır (kullanıcının kilit örneğine IPC gitmesin).

## Uyku öncesi kilit (kontrol listesi için)

**Karar (2026-10-16): uykudan önce kilit varsayılan AÇIK** — `[idle]
lock_before_sleep = true`. Gerekçe: GNOME, KDE, macOS ve Windows'ta varsayılan; kapalı olması
güvenlik sürprizi (kapağı kapatıp çantaya koyan, açınca masaüstünü açık bulur). Ayarlar'dan
"Uykudan önce kilitle" ile kapatılır. Boşta kilidinden (`lock`, varsayılan kapalı) ayrı anahtar;
"hiçbir boşta eylemi varsayılan açık değil" kullanıcı kararı bozulmaz (bu bir boşta eylemi değil).
`[idle] enabled = false` (hypridle kullanan) bunu da kapatır: çift inhibitör/çift kilit olmasın.

Akış (`services/idle/machine.rs`, saf, birim testli):
- `enabled && lock_before_sleep` iken logind uyku **gecikme** inhibitörü sürekli tutulur.
- `PrepareForSleep(true)` → kilitli değilse `Session.Lock()` → `Lock` sinyali → `nuxut-lock`.
  Uyku, kilit yüzeyi gerçekten örtene dek bekletilir: `nuxut-lock` `LockedHint = true`'yu
  ext-session-lock `locked` + tüm yüzeylerin ilk karesi sunulduktan sonra yazar (`--ready-fd`
  ile aynı an); kabuk bunu görünce inhibitörü bırakır.
- Zaman aşımı: `InhibitDelayMaxUSec` (okunamazsa 5 sn, en çok 30 sn) dolunca inhibitör yine de
  bırakılır ve uyarı loglanır — uyku asla süresiz engellenmez.
- Zaten kilitliyse inhibitör hemen bırakılır. logind inhibitörü vermediyse uyku beklemez
  (uyarı), kilit yine istenir.
- **Uyanınca kilit açık kalmaz:** bu uyku için istenen kilit kurulmadıysa (`LockedHint` hiç
  `true` olmadı: zaman aşımı, kilitleyici çöktü/reddedildi) `PrepareForSleep(false)`'ta
  `Session.Lock()` yeniden istenir. Kilitleyici hâlâ açılıyorsa ikinci istek ikinci kilit açmaz
  (`LockLauncher` kendi çocuğu çalışırken başlatmaz; nuxut-lock tek örnek). Kilit kurulup
  sonra kimlik doğrulamayla açıldıysa yeniden istenmez.

Sınırlar:
- `LockedHint` kullanılıyor, `--ready-fd` değil: kullanıcının kendi kilitleyicisi de
  (`listen_logind = false`) `LockedHint` yazıyorsa aynı bekleme çalışır; yazmıyorsa uyku zaman
  aşımına dek bekler ve uyanışta `Session.Lock()` bir kez daha gider.
- Zaman aşımında makine kilit kurulmadan uyuyabilir; uyanışta kısa bir an (kilit açılana dek)
  masaüstü görünebilir. logind'in `InhibitDelayMaxUSec`'i (varsayılan 5 sn) üst sınırdır.
- Gerçek logind + gerçek uyku sırası elle, kullanıcıyla doğrulanacak (özel bus testleri sırayı
  sahte logind'le doğrular).

## Testler

- `services::lock_launcher` birim: `decide` matrisi, test örneği kuralı, sahte `nuxut-lock`
  betiğiyle çalışırken ikinci başlatma yok / bitince yeniden başlatma, ikili yokken hata.
- `tests/it/idle_logind.rs::session_lock_signal_only_from_logind_for_our_session`: özel bus,
  sahte login1; başka oturumun `Lock`'u, `Unlock` ve sahte göndericinin (yayın + tek hedefli)
  `Lock`'u iletilmez, kendi oturumumuzunki iletilir.
- `ipc`: `lock` ayrıştırma.
- Canlı: izole sway + `dbus-run-session` + `examples/fake_logind` (Session.Lock yalnızca sinyal
  yayınlar) + fake-auth `nuxut-lock`: `msg lock` → kilit yüzeyi; logind'siz (sahte bus yok)
  doğrudan başlatma.

## İkinci tur (2026-10-16, c696288 sonrası "Kilitle hâlâ çalışmıyor")

Kullanıcının makinesi: tty'den `start-hyprland`, kabuk systemd kullanıcı servisi
(`nuxut-shell.service`), iki GPU, iki monitör. Bulunanlar ve düzeltmeler:

- **`nuxut-lock` kurulmuyordu (kök neden).** `install.sh` `BINS` listesinde ve `cargo build -p`
  satırında `nuxut-lock` yoktu; `LockLauncher` onu `current_exe` kardeşi olarak (birimde
  `ExecStart=$BIN/nuxut-shell`) arıyor, bulamayınca yalnızca `error!` logluyordu. Artık kurulur.
- **Sessiz başarısızlık yok.** Başlatılamazsa (`NotFound`, spawn hatası) ve kendi başlattığımız
  `nuxut-lock` 0 dışı kodla/sinyalle çıkarsa (ext-session-lock reddi = 1, çökme) kullanıcıya
  bildirim açılır (`shell-lock-*` iletileri; `LockLauncher::with_exit_sink`).
- **Sinyal bekçisi.** `Session.Lock()` iletildi ve dinleyici hazır diye sinyal beklenirken
  `LOCK_SIGNAL_GRACE` (1,5 sn) içinde kendi oturumumuzun `Lock` sinyali gelmez, oturum kilitli
  değilse ve kendi kilidimiz çalışmıyorsa `nuxut-lock` doğrudan başlatılır (uyarı loglanır).
  Sinyal başka oturuma gitse de kilit düşmez.
- **Oturum çözümü.** Servis olarak çalışan kabuğun PID'i hiçbir oturumda değil
  (`NoSessionForPID`). Önceden doğrudan `session/auto`'ya düşülüyordu; o kullanıcının "görüntü
  oturumu"dur ve birden çok oturumda (ör. ssh + tty) yanlışını seçebilir. Artık sıra: kendi PID
  → compositor PID'i (`hyprland.lock`) → `session/auto`. Hem `SessionService` (Lock/Terminate)
  hem `IdleLogind` (sinyal aboneliği) aynı sırayı kullanır (`compositor_pid` seçeneği).
- **İki GPU / iki monitör:** kodda çıkışa özgü bir hata bulunmadı; test compositor'ında iki
  çıkış (biri 1,5 ölçekli) ile denendi (aşağıda). Gerçek donanımda çökme/ret olursa artık
  bildirim ve günlük (çıkış kodu/sinyal) görünür.
- Açık: `nuxut-lock` `SetLockedHint`'i hâlâ `session/auto`'ya yazar (bilgi amaçlı; güvenlik ona
  dayanmaz).
