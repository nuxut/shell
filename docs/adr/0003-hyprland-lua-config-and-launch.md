# ADR-0003: Hyprland Lua ayarı ve kabuğun başlatılması

- Durum: kabul edildi (2026-10-10; kararlar 1-2-3 onaylı). Numaralar: `nuxut-apply` 0001/0002 (ortak dış-config katmanı), `hypr_config` 0003, `look` 0004, `displays` 0005.
- İlgili: `docs/design/hyprland-sync.md` (eşleme tablosu, kısayol modeli, Ayarlar sözleşmesi — geçerli kalır),
  kullanıcı yorumu 1, kararlar 1A, 2A, 3B (kullanıcı).

## Bağlam

Hyprland 0.55'te hyprlang ayar dili kullanımdan kaldırıldı; yeni dil Lua (`hyprland.lua`). Bu VM'de
0.56.2 çalışıyor; kullanıcının kendi ayarı hâlâ `hyprland.conf`. `c7a354e` yalnızca hyprlang
üretiyordu ve canlı uygulamada `hyprctl keyword` kullanıyordu: Lua ayarı altında ikisi de çalışmaz.
Ayrıca başlatma kararı `exec-once`'tan systemd kullanıcı servisine değişti (3B).

## Doğrulanmış bulgular (Hyprland v0.56.2)

Kaynaklar: wiki [Configuring/Start](https://wiki.hypr.land/Configuring/Start/),
[Basics/Autostart](https://wiki.hypr.land/Configuring/Basics/Autostart/); sistemdeki
`/usr/share/hypr/hyprland.lua` (örnek ayar) ve `/usr/share/hypr/stubs/hl.meta.lua` (API taslakları);
kaynak `github.com/hyprwm/Hyprland` etiketi `v0.56.2` (salt okunur klon). Satır numaraları o etikette.

1. **Hangi dosya** — `src/config/supplementary/jeremy/Jeremy.cpp` `getMainConfigPath`: sıra
   (a) güvenli kip → `<instance>/recoverycfg.lua`, (b) `--config/-c`, (c) Hyprland sürecinin
   `HYPRLAND_CONFIG` ortamı, (d) XDG aramasında (`$XDG_CONFIG_HOME`, `~/.config`, `$XDG_CONFIG_DIRS`,
   `/etc/xdg`) bulunan `hypr/hyprland.lua`, (e) yoksa `hypr/hyprland.conf`, (f) hiçbiri yoksa
   `hyprland.lua` **üretilir**. Yani `hyprland.lua` varsa `hyprland.conf` yok sayılır. Uzantı `.lua`
   ise Lua yöneticisi, değilse eski (hyprlang) yönetici (`src/config/ConfigManager.cpp` 15–70).
   Çalışma anında hangisi: `hyprctl -j status` → `configProvider: "lua" | "hyprlang"`
   (`src/helpers/SystemInfo.cpp`).
2. **Dahil etme (Lua)** — `package.path`'in başına `<ayar dizini>/?.lua;<ayar dizini>/?/init.lua`
   eklenir (`lua/ConfigManager.cpp` 534–546); `require("nuxut")` → `~/.config/hypr/nuxut.lua`.
   `require` sarmalanmıştır (`safeLuaRequire`, 312–360): modül **bulunamazsa** Lua hatası atar
   (ayarın geri kalanı çalışmaz; hiç bind kalmazsa "acil durum" bind'leri devreye girer, 783).
   Bulunan modülün hatası ise `configerrors`'a yazılır ve boş tablo döner. `require`'lanan dosyalar
   izlenir (`package.searchers[2]` kancası, 561–598) → değişince otomatik yeniden yükleme
   (`misc:disable_autoreload` kapalıysa, `shared/inotify/ConfigWatcher.cpp`).
3. **API** (`hl.meta.lua`): `hl.config({ general = {…}, decoration = { rounding, shadow = {…},
   blur = {…} }, animations = { enabled } })`; `hl.curve(ad, { type = "bezier", points =
   {{x1,y1},{x2,y2}} })` ve **`{ type = "spring", mass, stiffness, dampening }`** (yay eğrisi artık
   yerleşik; `LuaBindingsConfigRules.cpp` 337–385, `stiffness`/`dampening` ≥ 0.5);
   `hl.animation({ leaf, enabled, speed, bezier = ad | spring = ad, style })`;
   `hl.bind("SUPER + N", hl.dsp.exec_cmd(cmd), { locked, repeating, description })` → `HL.Keybind`
   (`:remove()`, `:set_enabled()`); `hl.unbind(tuş)`; `hl.on("hyprland.start" | "config.reloaded" |
   "hyprland.shutdown", fn)`; `hl.exec_cmd(cmd)`; `hl.get_config(anahtar)`.
4. **exec-once karşılığı** — `hl.on("hyprland.start", function() hl.exec_cmd("…") end)`: yalnızca
   açılışta, yeniden yüklemede değil (wiki Autostart; `LuaEventHandler.cpp` 165).
5. **Çalışma anında değişiklik** — `hyprctl keyword` Lua altında **çalışmaz**: "keyword can't work with
   non-legacy parsers. Use eval." (`src/debug/HyprCtl.cpp` 1160–1163). Yerine `hyprctl eval <lua>`
   (yalnızca Lua yöneticisi, zaman aşımı 250 ms; `evalRequest`). `dispatch` Lua altında
   `hl.dispatch(…)` sarmalayıcısıdır. `reload` argümanı yok sayar (yalnızca `full-reset` özel: ayar
   yolunu yeniden seçer — `.conf`→`.lua` geçişi için); `reload config-only` 0.56.2'de düz `reload`'dur
   (`reloadRequest`, 1260–1279).
6. **Bind'ler** — `hyprctl -j binds` her bind'in `modmask`, `key`, `description`, `dispatcher`, `arg`
   alanlarını verir: Lua ayarını ayrıştırmadan çakışma denetimi buradan yapılır; bizimkiler
   `description = "nuxut:<eylem>"` ile ayırt edilir.
7. **Ortam (systemd)** — Hyprland oturum açılışında kendisi `systemctl --user import-environment
   DISPLAY WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE XDG_CURRENT_DESKTOP QT_QPA_PLATFORMTHEME PATH
   XDG_DATA_DIRS` ve `dbus-update-activation-environment --systemd …` çalıştırır (`Compositor.cpp`
   785–797; `HYPRLAND_NO_SD_VARS` ile kapanır; iç içe/oturumsuz Hyprland'de yapılmaz). Bu çağrı
   **eşzamansızdır** (spawn); bizim başlatmamızla yarışabilir. Bu VM'de `graphical-session.target`
   **etkin değil** (uwsm yok): yalnızca `WantedBy=graphical-session.target` olan birim kendiliğinden
   başlamaz.
8. Dönüştürücüler (hyprlang → Lua): `github.com/EIonTusk/hyprlang2lua`,
   `github.com/Prateek-squadron/hyprconf2lua` — kullanıcının kendi dosyası için; Nuxut bunları
   çalıştırmaz (bkz. Karar 4).
9. **Yay eğrisi fiziği** — `hyprutils` (`src/animation/Spring.cpp` `advanceSpring`, commit 95983ee):
   kütle-yay-sönüm, `ω0 = √(k/m)`, `γ = c/(2m)`; adım gerçek zamanla (`steady_clock`), kesilen
   animasyonda hız korunur (`resetSpringState(preserveVelocity)`). `speed` yay eğrisinde
   **kullanılmaz** (`AnimatedVariable.cpp` `getCurveStep`), ama `hl.animation` `speed > 0` ister.
   Framework yayı (`response` T, `damping` ζ) birebir çevrilir: `m = 1`, `k = (2π/T)²`,
   `c = 2ζ·(2π/T)`; `speed = 1` yazılır. Böylece pencereler shell'le **aynı** yayla hareket eder.

## Kararlar

### 1. Ayar dosyası tespiti
Hyprland'in sırası taklit edilir: çalışan örneğin pid'i (`$XDG_RUNTIME_DIR/hypr/<imza>/hyprland.lock`
ilk satır) → `/proc/<pid>/cmdline` `-c` → `/proc/<pid>/environ` içinden **yalnızca**
`HYPRLAND_CONFIG` → XDG araması (`hyprland.lua` önce, sonra `.conf`). Çalışma anında
`hyprctl -j status` `configProvider` ile çapraz denetlenir; uyuşmazsa (ör. kullanıcı yeni
`hyprland.lua` yazdı, Hyprland henüz `.conf` ile çalışıyor) **diskteki** dosya esas alınır (sonraki
oturum onu kullanacak) ve canlı uygulama o oturum için yapılmaz, durum `pending_switch`.
`$XDG_CONFIG_DIRS`/`/etc/xdg` altındaki sistem dosyalarına asla yazılmaz (`unsupported`).
Güvenli kip (`recoverycfg.lua`) → hiçbir şey yapılmaz.

### 2. Lua: üretilen modül + tek dahil etme satırı
- Üretilen dosya: `<ayar dizini>/nuxut.lua` (Nuxut'un; her değişiklikte atomik yeniden yazılır).
- Dahil etme: `hyprland.lua` **sonuna** (1A: Nuxut kazanır) yorum + tek satır:
  `pcall(require, "nuxut")`. `pcall` şarttır: dosya silinir/bozulursa kullanıcının ayarı yine
  çalışır (Bulgu 2: bulunamayan modül tüm ayarı durdurur). Hata `nuxut.lua`'nın kendisinde ise
  modül içi `pcall` ile `hl.notification.create` bildirimine düşer (aşağıda).
- "Var mı" taraması (Lua, saf, sınırlı): `--` ve `--[[ … ]]` yorumları atlanır; `require "nuxut"`,
  `require("nuxut")`, `require('nuxut')`, `pcall(require, "nuxut")`, `dofile("…/nuxut.lua")` görülürse
  var; yalnızca yorum içindeyse "kullanıcı kapatmış". Kullanıcının kendi `require` ettiği modüller
  (`<dizin>/<ad>.lua`, `<ad>/init.lua`, `a.b` → `a/b.lua`) aynı sınırlarla (derinlik 8, 64 dosya,
  1 MiB) taranır; dinamik `require(değişken)` görülemez → tarama "eksik" sayılmaz ama dosya
  bulunamazsa satır eklenir (çift dahil etme zararsızdır: `require` ikinci kez yüklemez).
- Değer kaçışı (dize enjeksiyonu yok): üretici **yalnızca** şu türleri yazar — sayılar (`f32`
  sonlu, sınırlı, `{:.3}`), `true/false`, sabit enum adları (kodda literal), renk
  `"rgba(rrggbbaa)"` (yalnızca hex üretimi), bind tuşu (doğrulanmış `[A-Za-z0-9_]`/`code:n` +
  sabit değiştirici adları) ve komut dizesi. Her dize tek bir `lua_str()`'den geçer: `"` ile
  sarılır; `\\`, `"`, ve 0x20 altı / 0x7f / ASCII dışı her bayt `\ddd` ondalık kaçışla yazılır
  (Lua 5.4 kuralı; çok baytlı UTF-8 de bayt bayt kaçırılır). Komut dizesinde ayrıca kabuk için
  yalnızca `[A-Za-z0-9/._+-]` yol karakterleri kabul edilir (değilse yalın `nuxut-shell`). Uzun
  köşeli dize (`[[ ]]`) ve `..` birleştirme kullanılmaz.
- Modülün iskeleti: `local ok, err = pcall(function() hl.config({…}); hl.curve(…); hl.animation(…);
  hl.bind(…) end)`; `if not ok then hl.notification.create({…}) end` (bir Nuxut hatası kullanıcının ayarını
  durdurmaz). Başlangıç kancası `hl.on("hyprland.start", …)` (Karar 5).

### 3. Canlı uygulama
- Lua: dosya yazılır; Hyprland `require`'lanan dosyayı izlediği için kendisi yeniden yükler
  (Bulgu 2). `misc:disable_autoreload` açıksa (`hyprctl -j getoption misc:disable_autoreload`, açılışta
  bir kez okunur) yalnızca değer değişiminde `hyprctl eval` ile aynı `hl.config{…}`/eğri/animasyon
  bloğu (bind'siz; ≤ 64 KiB, üreticiden), bind değişiminde `reload`. `eval` metni dosyadakiyle aynı
  üreticiden gelir; ayrı string birleştirmesi yoktur.
- hyprlang (`.conf`, geçiş dönemi): mevcut yol (`[[BATCH]]keyword …`, `reload`), çünkü `.conf`
  altında `eval` yok. Her iki durumda `configerrors` okunur.
- Çakışma/ezilme raporu: bind'ler `hyprctl -j binds`'ten (Bulgu 6; bizimkiler açıklamadan
  ayıklanır). Ezilen değerler: ilk uygulamadan **önce** yönetilen anahtarların o anki değerleri
  `getoption` ile okunup anlık görüntüye (snapshot) yazılır — hem "Geri al" bildirimi hem rapor
  bunu kullanır (Karar 6). Lua dosyası ayrıştırılmaz.

### 4. `.conf` kullanıcıları: geçiş yolu
- Nuxut kullanıcının ayarını **dönüştürmez** (kullanıcının dosyası; dönüştürücüler deneysel).
- Geçiş dönemi boyunca ikili arka uç: tespit `.conf` ise `nuxut.conf` + `source` satırı
  (`c7a354e`'deki yol), `.lua` ise `nuxut.lua` + `pcall(require, "nuxut")`. Aynı ayar tek
  üreticiden iki biçime (`HyprBackend::{Lua, Hyprlang}`).
- Kullanıcı kendi ayarını Lua'ya taşıdığında (yeni `hyprland.lua` belirir) shell bunu açılışta ve
  ayar dizini değişiminde görür, `nuxut.lua`'yı üretir, satırı ekler, durum `pending_switch`
  (Hyprland bir sonraki oturumda geçer). Oturum başına bir kez bildirim: "yeniden giriş yapınca
  geçerli" + isteğe bağlı **Şimdi uygula** düğmesi (`msg hypr apply-switch`); yeniden yükleme
  komutunu Nuxut yalnızca bu düğme/komutla gönderir, kendiliğinden göndermez. Eski `nuxut.conf` + `source` satırı `hyprland.conf`'ta kalır (artık
  okunmuyor, zararsız); "Geri al" ikisini de temizler.
- Geri alınabilirlik: Nuxut'un kullanıcı dosyasındaki izi her zaman **tek blok**
  (`-- Nuxut …`/`# Nuxut …` yorumu + tek satır). "Geri al" = yalnızca o iki satırı kaldır (içerik
  karşılaştırmalı; satır elle değiştirilmişse dokunma, bildir) + üretilen dosyayı sil + durum
  işaretini "kullanıcı kaldırdı" yap. Ayrıca ilk dokunuştan önceki tam yedek (ortak katman) durur.

### 5. Başlatma: systemd kullanıcı servisi (3B)
`~/.config/systemd/user/nuxut-shell.service` (install.sh kurar):

```ini
[Unit]
Description=Nuxut masaüstü kabuğu
PartOf=graphical-session.target
After=graphical-session.target
StartLimitIntervalSec=30
StartLimitBurst=5

[Service]
Type=simple
ExecStart=%h/.local/bin/nuxut-shell
Restart=on-failure
RestartSec=1
# Compositor kapanınca kabuk 0 ile çıkar (yeniden başlatma döngüsü yok).
TimeoutStopSec=5

[Install]
WantedBy=graphical-session.target
```

- Başlatıcı Hyprland'in kendisi (graphical-session.target bu VM'de etkin değil; uwsm'li sistemlerde
  `WantedBy` zaten başlatır, ikinci `start` etkisizdir). Üretilen dosyadaki başlangıç kancası:
  `systemctl --user import-environment PATH WAYLAND_DISPLAY HYPRLAND_INSTANCE_SIGNATURE
  XDG_CURRENT_DESKTOP && systemctl --user start nuxut-shell.service || exec <yol>/nuxut-shell`.
  `import-environment` burada da yapılır çünkü Hyprland'in kendi içe aktarması eşzamansızdır
  (Bulgu 7) — yarış yok. systemd kullanıcı oturumu yoksa (`systemctl` başarısız) doğrudan
  başlatmaya düşer. Kapanışta `hl.on("hyprland.shutdown")` → `systemctl --user stop
  nuxut-shell.service` (en iyi çaba).
- Kabuk, Wayland bağlantısı compositor tarafından kapatılınca 0 ile çıkar (yeni; `Restart=on-failure`
  döngüsünü önler). Diğer hatalar ≠ 0 → systemd yeniden başlatır (30 sn'de en çok 5).
- Çift başlatma: (a) tek örnek IPC'si (zaten), (b) üretilen dosya kancayı yalnızca kullanıcının
  ayarında başka bir başlatıcı (`exec-once = nuxut-shell`, `hl.exec_cmd("…nuxut-shell…")`, uwsm
  `app -- nuxut-shell`) **yoksa** yazar, (c) birim etkinse (`systemctl --user is-enabled`) ve oturum
  hedefi etkinse kanca yalnızca `import-environment` yapar (başlatmayı systemd'ye bırakır),
  (d) systemd altında (`INVOCATION_ID`) çalışan kabuk kendi `LaunchMode`'unu `systemd` bildirir.
- `install.sh`: birimi `%h/.local/bin` yerine kurulum önekine göre `sed`'ler, `daemon-reload`;
  yalnızca `uwsm` varsa `enable` eder (graphical-session.target'ı o başlatır); yoksa etmez
  (hedef etkin değilse anlamsız; başlatma kancadan). `--uninstall`
  birimi durdurur/siler. Eski `exec-once` satırı olan `nuxut.conf` ilk açılışta yeniden üretilir.

### 6. Ortak dış-config katmanı (ADR-0001'e kadar ara yüz)
Bütün yazım noktaları tek bir `ExternalConfigWriter` özelliği arkasında toplanır:
`snapshot(path) -> SnapshotId` (ilk dokunuştan önce tam kopya), `write_owned(path, text)` (Nuxut'un
dosyası; atomik), `append_block(path, block) -> Applied` (kullanıcı dosyası; yedek + atomik +
yarış denetimi), `remove_block(path, block)` (geri al), `restore(SnapshotId)`. Bugünkü
`ensure::append_source_line` ve `sync::write_generated` bu özelliğin tek uygulamasına taşınır;
ADR'si gelince uygulama değişir, çağıranlar değişmez. Hyprland'e komutlar zaten
`HyprControl` arkasında (sahte testlerde).

### 7. İlk uygulama bildirimi (1A)
Satır ilk kez eklendiğinde: "Masaüstü ayarların Nuxut'a bağlandı — mevcut köşe, aralık, gölge ve
animasyon ayarların Nuxut'unkilerle değiştirildi." Eylem düğmesi **Geri al** (`nuxut:hypr-undo`) →
Karar 4'teki geri alma + `[compositor] ensure_source = false` (shell.toml'a ortak katman üzerinden).
Bildirim eylemi shell'in kendi bildirim sunucusunda yakalanır (D-Bus `ActionInvoked`).

## Sonuçlar
- `keyword` yalnızca `.conf` arka ucunda kalır; Lua'da dosya + Hyprland'in kendi izlemesi esastır.
- Lua tarafında kullanıcı ayarı ayrıştırılmaz; çakışma/ezilme bilgisi çalışan Hyprland'den alınır
  (test örneğinde Hyprland'e hiç gidilmediği için bu raporlar testte boş; birim testleri sahte
  `HyprControl` yanıtlarıyla).
- Test kuralı aynen: test örneği geçici `XDG_CONFIG_HOME` dışına yazmaz, Hyprland'e komut göndermez.
  Gerçek Hyprland davranışı (eval, otomatik yeniden yükleme, `pcall(require)`) için izole iç içe
  Hyprland (headless sway içinde, geçici ayar dizini, kendi imzası) ile doğrulama planlanır.
  Test örneği yalnızca `NUXUT_TEST_HYPR_CONTROL=1` ve `/run/user` dışındaki bir `XDG_RUNTIME_DIR`
  ile Hyprland'e komut gönderir (soket o dizinde: kullanıcının oturumuna yapısal olarak ulaşılamaz).
- Canlı iç içe test bu VM'de **yapılamıyor** (2026-10-10 yoklaması): virtio-gpu 3B'siz; ayrıcalıksız
  süreç render düğümünde GBM tamponu ayıramıyor (Mesa kms_swrast → `DRM_IOCTL_MODE_CREATE_DUMB`
  EPERM), iç içe Hyprland "Cannot open backend: no allocator available" ile düşüyor (pixman sway'de
  ise `zwp_linux_dmabuf_v1` yok). Not: `WAYLAND_DISPLAY` verilse de Hyprland önce DRM arka ucunu
  dener ve logind `TakeControl` çağırır (kullanıcının oturumunda EBUSY ile reddedildi);
  iç içe testte `LIBSEAT_BACKEND=<geçersiz>` + `HYPRLAND_NO_SD_VARS=1` + sahte `systemctl` şart.
  3B'li (virgl ya da gerçek GPU) bir makinede yoklama betiği aynı kalıpla çalışır.

## Kararlar (2026-10-10 — önerilenler onaylı)
1. "Geri al" yalnızca Nuxut satırını/bloğunu ve ürettiği dosyayı kaldırır (Karar 4); ilk dokunuştan
   önceki tam yedek geri yüklenmez (arada kullanıcının yaptığı değişiklikler korunur). Ortak
   snapshot katmanı (ADR-0001/0002) gelince yedek oradan erişilebilir kalır.
2. Kullanıcı kendi ayarını Lua'ya taşıdığında Nuxut çalışan oturuma kendiliğinden uygulamaz:
   `pending_switch` + oturum başına bir kez bildirim ("yeniden giriş yapınca geçerli") ve isteğe
   bağlı **Şimdi uygula** düğmesi. Eşdeğer komut: `nuxut-shell msg hypr apply-switch`.
3. systemd birimi yalnızca `uwsm` varsa etkinleştirilir (install.sh); yoksa üretilen dosyadaki
   kanca `systemctl --user start` ile başlatır.
