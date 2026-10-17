# Boşta kalma: karart → ekranı kapat → kilitle → uyu (t061)

Kullanıcı bir süre girdi vermeyince shell ekranı karartır, sonra kapatır, oturumu kilitler,
pildeyken makineyi uyutur; uykudan önce kilitler. Kahve modu (`msg idle inhibit`) hepsini
durdurur. hypridle'ın yaptığını shell kendi içinde, ayarlardan yönetilebilir biçimde yapar;
hypridle kullanan biri `[idle] enabled = false` ile çakışmayı önler.

**Kullanıcı kararı: hiçbir boşta eylemi varsayılan açık değil.** İstisna uykudan önce kilit
(`lock_before_sleep`, varsayılan AÇIK; karar 2026-10-16, aşağıda *Uyku*). Her aşamanın kendi anahtarı vardır,
hepsi `false` başlar; süreler saklanır ve anahtar açılınca makul değerle başlar.

## Aşamalar ve varsayılanlar

| Aşama | Anahtar / süre | Varsayılan | Ne yapar |
|---|---|---|---|
| Karart | `dim` / `dim_after` | kapalı / 300 sn | Her monitörde girdiyi geçiren yarı saydam örtü (Overlay katmanı, boş input bölgesi; 1 sn'de belirir, tema `shadow` rengi %50). Girdi gelince hemen kalkar. |
| Ekranı kapat | `screen_off` / `screen_off_after` | kapalı / 600 sn | Hyprland `dispatch dpms off` (ana döngü dışında); girdi gelince ve uyanınca `dpms on`. |
| Kilitle | `lock` / `lock_after` | kapalı / 600 sn | logind `Session.Lock()` (bar "Kilitle" ile aynı tek yol). Zaten kilitliyse (`LockedHint`) atlanır. `lock_after = 0`: boşta kilitlenmez. |
| Uyu | `suspend` / `suspend_after` | kapalı / 1800 sn | logind `Suspend(false)` (polkit sorusu yok); **yalnızca pildeyken** (`suspend_on_ac = false`). |

Süreler son girdiden itibaren ölçülür (hypridle gibi aşama başına ayrı sayaç). Sınır
`30..=14400` sn (4 sa; `lock_after` için `0` da geçerli); dışı kırpılır ve uyarılır. Aşamalar
birbirinden bağımsızdır: kilit karartmadan önce gelebilir. `enabled = false` ana anahtardır:
hiçbir aşama izlenmez, uyku öncesi kilit ve gecikme inhibitörü de yoktur.

`lock` yalnızca boşta kilittir ("Boşta kilitle"); uykudan önce kilit ayrı anahtardır
(`lock_before_sleep`, "Uykudan önce kilitle", varsayılan açık). 2026-10-16'ya dek `lock` ikisini
birden açıyordu ve varsayılan kapalı olduğundan kutudan çıktığı hâliyle uyku öncesi kilit yoktu.

**Kilit dinleyicisi.** `Session.Lock()` yalnızca o sinyali dinleyen bir kilitleyici varsa kilit
ekranı açar. 2026-10-16'dan beri (karar) kabuk oturumun `Lock` sinyalini boşta
ayarlarından bağımsız her zaman dinler ve `nuxut-lock`'u başlatır; logind kilidi iletemezse
doğrudan başlatır (`docs/design/lock-trigger.md`, `[lock] listen_logind`). Öncesinde tetikleyici
kapalıydı ve `lock = true` kendi başına kilit ekranı göstermiyordu.

## Kim ne zaman "boşta"?

- **Compositor (ext-idle-notify-v1):** her etkin aşama için ayrı bildirim nesnesi (süre =
  aşamanın süresi). `idled` → aşama boşta, `resumed` → girdi geldi. `get_idle_notification`
  Wayland **idle inhibitor'larına saygı duyar** (video oynatan tarayıcı, mpv): inhibitor varken
  compositor `idled` göndermez. Bağlama framework'ün işi (Wayland runtime; shell'de protokol
  kopyası yok) — aşağıda *Compositor bağlaması*. Compositor desteklemiyorsa kaynak
  `unsupported` (sessiz yedek yok), boşta aşamaları çalışmaz; uyku öncesi kilit, engeller, IdleHint temizliği ve `msg idle` çalışır.
- **Shell düzeyinde engeller** (hepsi aynı etkiyi yapar):
  - `manual`: kahve modu, `msg idle inhibit|uninhibit|toggle` (kalıcı değil; yeniden başlatınca kalkar).
  - `media`: bir MPRIS oynatıcısı `Playing` (`media_inhibits = true`). Mevcut `media`
    servisinin oynatıcı listesinden; yeni D-Bus bağlantısı yok.
  - `logind`: logind'de `idle` blok inhibitörü var (`BlockInhibited` içinde `idle`;
    `systemd-inhibit --what=idle`, GNOME uygulamaları). Her zaman saygı duyulur.
- Engel başlayınca karartma kalkar, ekranlar açılır, IdleHint düşer. Engel bitince bildirim
  nesneleri **yeniden kurulur**: sayaç engelin bittiği andan başlar (yoksa 20 dk önce
  duraklatılan müzikten sonra hemen kilitlenirdi; noctalia da böyle yapar).

## logind IdleHint

Etkin bir aşama boştayken (engel yokken) shell oturumun `IdleHint`'ini `true` yazar
(`Session.SetIdleHint`), girdi/engel/uyanışta `false`. logind'in `IdleAction`'ı ve oturum
izleyicileri bunu kullanır. İstek kalıcıdır: logind yeniden gelince yeniden yazılır; ilk
bağlantıda `false` yazılması çöken eski bir örneğin bıraktığı `true`'yu temizler.

## Uyku (logind)

- `lock_before_sleep = true` (ve `enabled`) iken shell logind'den `Inhibit("sleep", "Nuxut", …, "delay")`
  gecikme inhibitörü tutar; aksi hâlde hiç tutmaz (uykuyu geciktirmenin sebebi yok).
  Koşul `enabled && lock_before_sleep` (eskiden `lock`'a bağlıydı).
- `PrepareForSleep(true)`: kilit açık ve oturum kilitli değilse `Session.Lock()`; inhibitör
  oturum `LockedHint = true` olunca (nuxut-lock bunu sunum kapısından sonra yazar) ya da
  `InhibitDelayMaxUSec` dolunca (en çok 30 sn; okunamazsa 5 sn) bırakılır — uyku asla süresiz
  engellenmez (zaman aşımı uyarıyla loglanır). Zaten kilitliyse hemen bırakılır. logind
  inhibitörü vermediyse uyku beklemez; kilit yine istenir.
- `PrepareForSleep(false)` (uyanış): ekranlar açılır (`dpms on`), karartma kalkar, bildirimler
  yeniden kurulur, kilit açıksa inhibitör yeniden alınır. **Uyanınca masaüstü açık kalmaz:**
  uykudan önce istenen kilit kurulmadıysa (zaman aşımı, inhibitör yok, kilitleyici çöktü;
  `LockedHint` hiç `true` olmadı) `Session.Lock()` yeniden istenir. Kurulup uykuda/uyanışta
  kimlik doğrulamayla açıldıysa yeniden istenmez.
- Oturum nesnesi: `GetSessionByPID(kendi PID)`, olmazsa (systemd kullanıcı servisi oturum
  dışındadır) `session/auto`'nun `Id`'si → `GetSession`. Sinyaller yalnızca `org.freedesktop.login1`
  adının güncel benzersiz sahibinden kabul edilir (taklit koruması, `nuxut_dbus`).
- logind düşüp gelirse durum sıfırlanır, yeniden okunur; istenen gecikme inhibitörü ve
  IdleHint yeniden uygulanır.

## Komutlar

```text
nuxut-shell msg idle status                 # tek satır JSON (aşağıda)
nuxut-shell msg idle inhibit|uninhibit|toggle
```

`status`: `{"enabled":true,"source":"compositor","watching":["dim"],"logind":true,"manual":false,"inhibited":false,
"inhibitors":[],"dimmed":false,"screens_off":false,"locked":false,"lock":false,
"sleep_delay":false,"sleep_delay_wanted":false,"idle_hint":false,"stages":{"dim":null,
"lock":null,"screen_off":null,"suspend":null},"on_ac":true}` (`stages`: saniye, `null` = kapalı;
`sleep_delay` logind'de gerçekten tutulan inhibitör, `sleep_delay_wanted` istenen;
`source` `compositor` (ext-idle-notify-v1 var) ya da `unsupported`; `watching` compositor'da
izlenen aşamalar).
Arayüz (2026-10-15): elle engel `ShellModel::idle_inhibit`te durur (yalnızca
`idle_command` değiştirir, değişince `model_changed`); widget'lar `Action::IdleInhibit(bool)` döndürür
— `msg idle inhibit|uninhibit` ile aynı yol. Kullananlar: hızlı anahtarlar kartı (`toggles`,
"Uyanık tut") ve bar'daki uyanık tut ikonu.

## Şema (`[idle]`, `since = 2`)

Başlık/açıklamalar ve doğrulama uyarısı katalogdan (`locales/*.ftl`, `shell-config-idle-*`;
ADR-0010). Bunun için `SettingSchema.title`/`description` `Cow<'static, str>` oldu (eski
`&'static str` çağıranlar değişmeden derlenir, JSON/TOML çıktısı aynı). Birim `s`.

```toml
[idle]
enabled = true            # ana anahtar (hypridle kullanıyorsan false)
dim = false
dim_after = 300           # sn, 30..=14400
screen_off = false
screen_off_after = 600
lock = false              # boşta kilitle
lock_after = 600          # 0 = boşta kilitleme
lock_before_sleep = true  # uykudan önce kilitle (varsayılan AÇIK)
suspend = false
suspend_after = 1800
suspend_on_ac = false     # prizdeyken de uyu
media_inhibits = true     # medya oynarken bekle
```

## Güvenlik

- Test örneği (`NUXUT_SHELL_INSTANCE` ayarlı) `DBUS_SYSTEM_BUS_ADDRESS` açıkça sahte bir bus'a
  yönlenmemişse logind bağlantısı kurmaz: gerçek sistemde inhibitör almaz, IdleHint yazmaz,
  kilitlemez, uyutmaz. Ekran kapatma yalnızca Hyprland istemcisi varsa (izole sway testinde yok).
- Aşama eylemleri (`Lock`, `Suspend`) bir boşta dönemi içinde en çok bir kez istenir; girdi
  gelince sıfırlanır. Etkiler fark olarak üretilir (aynı girdi iki kez etki üretmez; rastgele
  dizi testi değişmezleri denetler).
- logind'den gelen metinler sınırlı (`BlockInhibited` 1 KiB, oturum yolu `MAX_PATH`, değişen
  özellik sayısı 64); bloklayan IO (D-Bus, Hyprland socket1) ana thread dışında.

## Compositor bağlaması (framework `d0e07e8`)

Framework ext-idle-notify-v1'i bağlar: `cx.idle_support()`, `cx.idle_watch(süre)` (hep
`IdleInhibitors::Respect`), `cx.idle_set_timeout(id, süre)`, `cx.idle_unwatch(id)`,
`App::idle(cx, id, idle)`. Eşleme `services::idle::watches::Watches` (Wayland'den bağımsız,
sahte arka uçla testli):

- `Effect::Watch(aşamalar)` → kalkan aşamanın izlemesi bırakılır, yeni aşamaya izleme kurulur,
  kalan aşama `idle_set_timeout` ile yeniden kaydedilir (süre aynı olsa da: protokolde sayaç
  sıfırlama yok, framework nesneyi yeniden yaratır; `Watch`'ın "sayaçlar şimdiden" anlamı).
- `App::idle(id, ..)` → kimlik aşamaya çevrilir, `IdleMachine::idle`'a verilir; bırakılmış
  kimliğin geç olayı yok sayılır.
- `idle_watch` hata dönerse (compositor desteklemiyor) sessiz yedek yok: tüm izlemeler bırakılır,
  `source: "unsupported"`. Hiç aşama istenmediyse (varsayılan) `source` compositor'ın o anki
  desteğidir.
- Sınır: notifier global'i shell açıldıktan sonra ilan edilirse izlemeler bir sonraki
  `Effect::Watch`'ta (ayar değişimi, engel bitişi, uyanış) kurulur.

Doğrulama: izole headless sway'de `dim = true, dim_after = 30` → girdi yokken ~30 sn'de örtü
(`dimmed: true`), sanal fare hareketiyle kalkar, `dim = false` canlı yüklenince izleme bırakılır
(`watching: []`). Hyprland'de (eylemler kapalı, kısa süreli test örneği) `source: "compositor"`.
