# ADR 0005: Ekranlar (monitör yapılandırması)

- Durum: kabul edildi (K1–K8 önerilenlerle onaylı, 2026-10-10). Uygulama: §13.
- Pano: t087. Kullanıcı hedefi: "gerçek tam bir DE'nin TÜM özellikleri".
- İlgili: `docs/design/hyprland-sync.md`, `nuxut-apply`'in snapshot/uygulama ADR'si,
  Ayarlar'ın Ayarlar → Ekranlar sayfası.

## 1. Bağlam

Kullanıcı Ayarlar'da **Ekranlar** sayfasını görür: çözünürlük, yenileme hızı, ölçek, sürükle-bırak
yerleşim, döndürme, yansıtma, VRR, birincil monitör, monitörü açma/kapama. "Hyprland ayarları"
diye bir yer yoktur; Hyprland'i ayarlamak shell'in işidir: değişiklik **çalışma anında uygulanır**,
kullanıcı onaylayınca **kalıcı** olarak üretilen dosyaya yazılır.

### 1.1 Doğrulanmış gerçekler (Hyprland 0.56.2, bu VM)

Kaynak: `/usr/share/hypr/stubs/hl.meta.lua`, `/usr/share/hypr/hyprland.lua`,
`/usr/include/hyprland/src/config/**`, Hyprland v0.56.2 `src/debug/HyprCtl.cpp` ve
`src/output/Monitor.cpp`, wiki.hypr.land, ve bu ADR için yapılan yalıtılmış denemeler (§9).

| Konu | Gerçek |
|---|---|
| Config sağlayıcıları | İki ayrı yönetici: `hyprlang` (eski `.conf`, "legacy") ve `lua` (`hyprland.lua`). `hyprctl status` → `configProvider: hyprlang\|lua`. **Bu VM'de kullanıcı hâlâ `hyprlang`** (`~/.config/hypr/hyprland.conf`); `.conf` kullanımdan kalkıyor ama çalışıyor. İkisi de desteklenmeli. |
| Lua monitör tanımı | `hl.monitor({ output = "DP-1" \| "desc:…" \| "", mode = "WxH@R" \| "preferred" \| "highres" \| "highrr", position = "XxY" \| "auto" \| "auto-right" …, scale = number \| "auto", transform = 0..7, mirror = "<çıkış>", vrr = 0..3, disabled = true, bitdepth, cm, sdr_*, reserved_area, icc, … })`. `output` zorunlu; bilinmeyen alan hatadır (`hl.monitor: unknown field 'bogus'`). |
| Çalışma anında değişiklik | Lua altında `hyprctl keyword …` **reddedilir**: `keyword can't work with non-legacy parsers. Use eval.` Lua'da `hyprctl eval '<lua>'` (zaman aşımı 250 ms); legacy'de `eval` reddedilir (`eval is only supported with the lua config manager`), `keyword monitor <kural>` kullanılır. **Arka uç config sağlayıcısına göre seçilmeli.** |
| eval hata biçimi | `error: [string "…"]:1: unexpected symbol near <eof>`, `error: …: hl.monitor: unknown field …`; başarı `ok`. `hyprctl repl '<lua>'` değer döndürür (`return #hl.get_monitors()` → `1`). |
| Lua zamanlayıcı | `hl.timer(fn, { timeout = ms, type = "oneshot" })` çalışıyor; dönen nesnede `:set_enabled(false)` iptal ediyor (denendi). Ölü adam anahtarı (§5.3) için kullanılabilir. |
| Statik doğrulama | `Hyprland --verify-config -c <dosya.lua>` compositor açmadan sözdizimi + alan denetimi yapar (denendi; `scale = 1.37` gibi **çalışma anı** hatalarını yakalamaz). |
| `hyprctl -j monitors all` | `name` (bağlayıcı: `DP-1`, VM'de `Virtual-1`), `description` (= make + model + serial, kısa), `make`, `model`, `serial` (VM'de boş!), `width/height` (piksel, döndürmesiz mod), `refreshRate`, `x/y` (mantıksal), `scale`, `transform` (0..7), `vrr` (**etkin adaptive sync durumu**, yapılandırılan kip değil), `disabled`, `mirrorOf` (**monitör id'si** ya da `"none"`; ad değil), `availableModes` (`"1920x1080@60.00Hz"` biçiminde dizgiler), `physicalWidth/Height` (mm), `focused`, `dpmsStatus`, `currentFormat`. `j/monitors` (all'sız) kapalıları vermez. |
| Ölçek geçerliliği | Hyprland, `piksel_boyu / ölçek` tam sayı değilse ölçeği "geçersiz" sayar, `round(ölçek·120)` çevresinde 1/120 adımla en yakın temiz böleni arar, bulamazsa hata bildirimi gösterir (`debug:disable_scale_checks` kapatır). `scale = "auto"`: PPI > 200 → 2, > 140 → 1.5, değilse 1. eval geçersiz ölçeği **reddetmez** (`ok` döner); düzeltme uygulama anında olur. |
| Koordinatlar | `position` mantıksal pikseldir: bir monitörün mantıksal boyu = piksel / ölçek, `transform` 1/3/5/7'de en-boy yer değiştirir. |
| Yansıtma | `mirror` hedef çıkışın **adını** alır; yansıyan monitörün workspace'leri yedek monitöre taşınır, konumu kaynağınkine eşitlenir. |
| Son monitörü kapatma | Hyprland engellemez; odak durumunu sıfırlar (siyah ekran, girdi hedefsiz). **Shell engellemeli.** |
| Hotplug | socket2 `monitoradded(v2)/monitorremoved(v2)` (v2 açıklamayı taşır); Lua'da ayrıca `hl.on("monitor.added" \| "monitor.removed" \| "monitor.layout_changed", fn)`. |
| Birincil monitör | Hyprland'de "birincil" kavramı yok. Karşılıkları: `cursor:default_monitor` (açılışta imleç/odak), workspace kuralı (`workspace 1` → monitör), shell'in varsayılan OSD/bildirim monitörü. |

### 1.2 Bugünkü kod

`src/services/hyprland.rs`: `HyprlandClient` (socket1, 1 sn zaman aşımı, 4 MiB cevap sınırı,
`request` yalnızca sabit komutlarla), `Monitor` (id, name, description, x, y, width, height,
scale, aktif/özel workspace, focused, disabled), sıcak yol `SNAPSHOT_COMMAND =
[[BATCH]]j/monitors;j/workspaces;j/activewindow;j/clients`, socket2 okuyucu `monitoradded/
monitorremoved/configreloaded` olaylarında yeniden sorgular. `src/ipc.rs`: satır tabanlı saf
komut ayrıştırıcı (shell tarafı `MAX_LINE = 512`, `MAX_TOKENS = 8`); framework runtime IPC'si
istek satırını **4096 bayt**, cevabı **64 KiB** ile sınırlar (`nuxut-wayland/src/ipc.rs`).

## 2. Karar (özet)

1. Yeni saf servis `src/services/displays/` (UI'dan bağımsız, testli): model, doğrulama,
   yerleşim, ölçek hesabı, kural üretimi (Lua + legacy), onay durum makinesi, profil eşleme.
   Hyprland'e yalnızca dar bir `DisplayControl` trait'i üzerinden konuşur (gerçek: socket1;
   test: sahte).
2. Monitör kimliği bağlayıcı adı değil **`DisplayId` = make + model + serial** (bkz. §3.1).
3. Çalışma anında uygulama config sağlayıcısına göre: Lua → `eval hl.monitor{…}`; legacy →
   `[[BATCH]]keyword monitor …`. Sağlayıcı açılışta ve `configreloaded`'da `status`'tan okunur.
4. **Riskli her değişiklik onaylıdır**: shell kendi katmanında (Ayarlar'da değil) 15 sn geri
   sayımlı "Bu ekran ayarları korunsun mu?" penceresini **tüm etkin monitörlerde** gösterir;
   Esc / zaman aşımı / pencere görünmüyorsa = geri dönüş. Shell çökerse bile geri dönüşü
   Hyprland'in yanında yaşayan bir **ölü adam anahtarı** yapar (§5.3).
5. Kalıcılık yalnızca onaydan sonra: doğruluk kaynağı `~/.config/nuxut/displays.toml`
   (`nuxut-apply`'in atomik yazım + snapshot katmanından geçer); Hyprland'in okuduğu dosya bundan
   **üretilir** (Lua: `nuxut/displays.lua`, legacy: `nuxut-displays.conf`), `hypr_config`'ın Nuxut
   modülü onu içerir (§6).
6. Hotplug: onaylanan her yapılandırma "bağlı monitör kümesi"ne göre saklanır ve aynı küme
   yeniden bağlandığında otomatik uygulanır (GNOME `monitors.xml` davranışı). Adlandırılmış
   profiller (ev/ofis) ve el ile geçiş sonraki adım.
7. IPC: `nuxut-shell msg displays list|status|apply|confirm|revert|identify` (§7).
8. Test: saf birim testleri + sahte socket1 + Hyprland `--verify-config` + yalıtılmış iç içe
   Hyprland (headless kwin içinde) protokol testleri; görüntü üretimi bu VM'de mümkün değil (§9).

## 3. Model

### 3.1 Kimlik

```rust
pub struct DisplayId { make: String, model: String, serial: String }   // her biri ≤ 64 kr, kontrol kr. yok
```
- Eşleme anahtarı `make|model|serial`. Hyprland kural dosyasına `output = "desc:<description>"`
  yazılır (description = Hyprland'in kısa açıklaması; make+model+serial'dan türetilir).
- **Serial boşsa ya da aynı kimlikten iki monitör bağlıysa** (VM'de serial boş; aynı model iki
  ofis monitörü): kimlik bağlayıcı adıyla tamamlanır (`DisplayId + connector`); kural `output =
  "<bağlayıcı>"` ile yazılır. Ayarlar'a `id_stable: false` gönderilir; UI "bu monitör kablo
  çıkışına göre tanınıyor" ipucu gösterebilir.
- Legacy `monitor=` satırı virgülle bölündüğü için açıklamasında `,` olan monitörler legacy'de
  bağlayıcı adıyla yazılır. Lua'da açıklama güvenli kaçışla (§7.4) yazılır.

### 3.2 Monitör ayarı

```rust
pub struct DisplayConfig {
    id: DisplayId,
    enabled: bool,
    mode: Mode,               // { width: u32, height: u32, refresh_mhz: u32 } — availableModes'tan
    position: (i32, i32),     // mantıksal piksel, sol üst
    scale_120: u32,           // ölçek × 120 (Hyprland/wp_fractional_scale adımı; f64 eşitlik sorunu yok)
    transform: Transform,     // Normal, R90, R180, R270, Flipped, FlippedR90, FlippedR180, FlippedR270 (0..7)
    mirror_of: Option<DisplayId>,
    vrr: Vrr,                 // Off(0) | On(1) | Fullscreen(2) | FullscreenMedia(3)
}
pub struct DisplayLayout { displays: Vec<DisplayConfig>, primary: Option<DisplayId> }   // ≤ 16 monitör
```
`vrr` ve `mirror_of` Hyprland'in JSON'undan güvenilir okunamaz (`vrr` yalnızca etkin durum,
`mirrorOf` id): yapılandırılan değerler `displays.toml`'dan, yoksa varsayılandan gelir.

### 3.3 Doğrulama (saf, `validate(&DisplayLayout, &Hardware) -> Result<(), Vec<Issue>>`)

- En az bir monitör **etkin ve yansıtma olmayan** olmalı (tüm monitörleri kapatma engeli;
  yansıtma zinciri ile "hepsi birbirini yansıtıyor" da reddedilir).
- `mode` o monitörün `availableModes` listesinde olmalı (yenileme ±0,05 Hz toleransla). Özel kip
  (CVT) bu sürümde yok.
- Ölçek: `0.5 ≤ s ≤ 4` ve `w_px·120 % scale_120 == 0 && h_px·120 % scale_120 == 0`
  (Hyprland'in "temiz bölen" kuralının tam sayı karşılığı). Geçersizse hata en yakın geçerli
  ölçeği önerir; shell asla Hyprland'e geçersiz ölçek göndermez (onun düzeltmesi sessiz ve
  bildirimlidir).
- Yerleşim (yansıtma olmayan etkin monitörler, mantıksal dikdörtgenler): **çakışma yok**, ve
  dikdörtgenler kenar/köşe temasıyla **bağlı** bir grafik oluşturur (boşlukla kopuk ada yok;
  fare geçemez). Shell kaydırmaz, reddeder; hizalamayı (yapışma) Ayarlar yapar.
- Normalleştirme: en küçük x ve y 0'a kaydırılır (Hyprland negatif koordinatı kabul eder ama
  XWayland ve bazı istemciler sorun çıkarır).
- `mirror_of` bağlı ve etkin, kendisi yansıtma olmayan bir monitörü göstermeli.
- `primary` etkin, yansıtma olmayan bir monitör olmalı; yoksa sol-üstteki seçilir.

### 3.4 Kesirli ölçekte yerleşim

Mantıksal boy = `(w_px·120/scale_120, h_px·120/scale_120)`, transform 1/3/5/7'de takas. Ayarlar'ın
tuvali yalnızca mantıksal dikdörtgenlerle çalışır; shell `list` cevabında her monitörün mantıksal
boyunu hazır verir ki iki taraf ayrı hesaplamasın. Kip/ölçek/döndürme değişince komşular
**itilmez**; Ayarlar (GNOME gibi) yeni dikdörtgeni yerinde bırakıp çakışma/kopukluk varsa en
yakın kenara yapıştırır, shell yine doğrular.

## 4. Çalışma anında uygulama

`DisplayControl` (trait; gerçek uygulaması `HyprlandClient` üzerinde):
```rust
fn provider(&self) -> Result<Provider, _>;                // status → Lua | Legacy
fn read(&self) -> Result<Hardware, _>;                    // j/monitors all (+ availableModes)
fn apply(&self, rules: &[MonitorRule]) -> Result<(), _>;  // Lua: tek eval; Legacy: [[BATCH]]keyword monitor …;
fn arm_guard(&self, …) / disarm_guard(&self, …);          // §5.3
```
- Kurallar **tüm monitörler için tek seferde** gönderilir (Lua: tek `eval`, içinde sıralı
  `hl.monitor{…}`; legacy: tek `[[BATCH]]`). Önce kapatılacaklar değil, önce **açılacaklar**
  uygulanır (ara durumda tüm monitörlerin kapalı kalmaması için); sonra yeniden konumlananlar,
  en son kapananlar.
- Birincil: `cursor:default_monitor` (Lua `hl.config{cursor={default_monitor=…}}`, legacy
  `keyword cursor:default_monitor …`) + shell içi varsayılan monitör. Workspace 1'i bağlamak
  `workspaces` ile konuşulacak (açık karar K6).
- Uyguladıktan sonra shell socket2 olaylarını bekler (≤ 2 sn), `j/monitors all`'ı yeniden okur ve
  **istenenle karşılaştırır** (kip, ölçek, konum, transform, etkinlik). Uyuşmazlık (Hyprland kipi
  düşürdü, ölçeği düzeltti) → otomatik geri dönüş + Ayarlar'a `error: not_applied` ayrıntısı.
- `HyprlandClient::request` "yalnızca sabit komutlar" kuralı korunur: eval/keyword metni yalnızca
  doğrulanmış tipli modelden üretilir; dışarıdan gelen hiçbir metin Hyprland'e aktarılmaz.
  `Monitor` yapısı bar'ın sıcak yolunda kalır; `displays` kendi `MonitorFull` tipini ve ayrı
  `j/monitors all` isteğini kullanır (yalnızca istek üzerine, sıcak yola maliyet yok).

## 5. Güvenlik: onay ve geri dönüş

### 5.1 Hangi değişiklik onay ister

Onaylı: kip/yenileme, ölçek, döndürme, açma/kapama, yansıtma, VRR. Onaysız (doğrudan uygulanır ve
kalıcılaşır; `nuxut-apply` snapshot'ı ile geri alınabilir): yalnızca konum ve yalnızca birincil
değişimi (ekranı karartamaz). Bkz. açık karar K2.

### 5.2 Durum makinesi (shell'de, saf ve testli)

```
Idle --apply(ok)--> Pending{token, previous, proposed, deadline=now+15s}
Pending --confirm(token)--> persist(proposed) --> Idle
Pending --revert(token) | Esc | timeout | hotplug | shell kapanışı--> apply(previous) --> Idle
Pending --apply--> error: pending_exists
```
- `previous` = **kalıcı** son yapılandırma (displays.toml'daki), çalışma anı durumu değil.
- Zamanlayıcı shell'dedir: Ayarlar uygulaması kapansa/çökse/donsa da geri döner.
- Onay penceresi shell'in kendi layer-shell yüzeyidir (overlay katmanı, klavye Exclusive),
  **değişiklikten sonra etkin olan tüm monitörlerde** aynı anda görünür (değişen monitör siyah
  olabilir; hangisinin görünür olduğunu bilemeyiz). İçerik: "Ekran ayarları korunsun mu?",
  geri sayım (15 → 0), "Geri al" (varsayılan odak) ve "Koru". Enter = odaktaki düğme, Esc = geri
  al. Bileşen framework'ün `Dialog`'u; yeni bileşen gerekmez (gerekirse framework'e).
- Pencere herhangi bir monitörde 2 sn içinde çizilemezse (yüzey configure gelmedi) beklemeden
  geri dönülür.
- Süre `shell.toml [displays] confirm_seconds` (5..=60, varsayılan 15), şemada.

### 5.3 Ölü adam anahtarı (shell çökerse)

Shell zamanlayıcısı ana yoldur; ikinci emniyet Hyprland tarafındadır ve shell'den bağımsız yaşar:
- **Lua:** apply ile aynı `eval` içinde `NUXUT_DISPLAY_GUARD = hl.timer(function() <previous
  kuralları> end, { timeout = (confirm_seconds + 5)·1000, type = "oneshot" })`; confirm/revert
  `NUXUT_DISPLAY_GUARD:set_enabled(false)` gönderir. (Bu VM'de denendi: zamanlayıcı tetikleniyor,
  iptal çalışıyor.)
- **Legacy:** zamanlayıcı yok. Hyprland'e `dispatch exec nuxut-shell display-guard --token T
  --deadline D` gönderilir (Hyprland'in çocuğu; shell ölse de yaşar). Bekçi süre dolunca
  `$XDG_RUNTIME_DIR/nuxut/displays-pending-<örnek>.json` hâlâ aynı token ile duruyorsa `hyprctl
  reload` yapar (dosya hâlâ eski = kalıcı yapılandırmayı içerdiği için tam yeniden yükleme geri
  dönüştür) ve dosyayı siler. Confirm sırası: önce kalıcı yaz, sonra pending dosyasını sil.
- Shell açılışında pending dosyası varsa ve süresi dolmuşsa geri dönüş uygulanır.

### 5.4 Diğer korumalar

- Hotplug Pending sırasında olursa geri dönülür (yapılandırma artık geçersiz olabilir).
- `hypr_config`'ın canlı uygulaması bu süreçte **`reload config-only`** kullanmalı (monitörleri yeniden
  yüklemez); tam `reload` Pending'i fiilen geri alır — zararsız ama shell bunu `configreloaded`
  ile görüp Pending'i "geri alındı" olarak kapatır.
- Kilit ekranı açıkken `apply` reddedilir (`error: locked`).

## 6. Kalıcılık

### 6.1 Doğruluk kaynağı: `~/.config/nuxut/displays.toml`

Ayrı dosya (shell.toml değil): makineye özgü donanım, insanlar elle değil Ayarlar akışı yazar,
küme profilleri tutar. Yazım ve geri alma `nuxut-apply`'in ortak katmanından (atomik yazım, ayar başına
snapshot, undo). Taslak:
```toml
version = 1
[[set]]                         # bağlı monitör kümesi başına bir kayıt (en çok 32; LRU)
key = ["Dell Inc.|DELL U2720Q|ABC123", "Red Hat, Inc.|QEMU Monitor|"]   # sıralı DisplayId'ler
primary = "Dell Inc.|DELL U2720Q|ABC123"
[[set.display]]
id = "Dell Inc.|DELL U2720Q|ABC123"
connector = "DP-1"              # yalnızca id_stable=false iken eşlemede kullanılır
enabled = true
mode = "3840x2160@59.997"
position = [0, 0]
scale = 1.5                     # okurken ×120'ye yuvarlanıp doğrulanır
transform = 0
vrr = "off"                     # off|on|fullscreen|fullscreen-media
mirror_of = ""                  # boş = yansıtma yok
```
Bilinmeyen anahtar yok sayılır, sınır dışı değer o monitör kaydını geçersiz kılar (log), dosya
≤ 256 KiB.

### 6.2 Üretilen Hyprland dosyası (`hypr_config` modülüyle sözleşme)

- `displays` yalnızca **saf bir üretici** sağlar: `displays::render(&DisplaySet, Provider) ->
  String`. Dosyayı yazmak, Nuxut modülüne bağlamak ve config sağlayıcısını bulmak `hypr_config`'ın
  mevcut mekanizmasıdır (atomik yazım, başlıkta "elle düzenlemeyin").
- Konum: Lua → `<hypr>/nuxut/displays.lua` (`hypr_config`'ın ana Nuxut modülü `require` /
  `dofile` ile yükler); legacy → `<hypr>/nuxut-displays.conf` (`nuxut.conf` içinden `source`).
  Ayrı dosya: monitör yazımı tema/kısayol üretimiyle yarışmaz ve kendi sıklığında değişir.
- İçerik yalnızca Nuxut'un **yönettiği** monitörlerin (o anki kümede kaydı olanların) kurallarını
  taşır; catch-all (`output = ""`) yazılmaz — kullanıcının kendi varsayılanı/`monitors.conf`'u
  bilinmeyen monitörler için geçerli kalır. Kullanıcının aynı monitör için kendi kuralı varsa
  `hypr_config` taraması `managed_overrides` gibi raporlar (Ayarlar: "monitors.conf:3'teki kuralı geçersiz
  kılıyor").
- Örnek (Lua):
  ```lua
  -- Nuxut tarafından üretildi; elle düzenlemeyin (Ayarlar → Ekranlar).
  hl.monitor({ output = "desc:Dell Inc. DELL U2720Q ABC123", mode = "3840x2160@60.00", position = "0x0", scale = 1.5, transform = 0, vrr = 0 })
  hl.monitor({ output = "Virtual-1", mode = "1920x1080@60.00", position = "2560x0", scale = 1, transform = 0, vrr = 0 })
  hl.config({ cursor = { default_monitor = "DP-1" } })
  ```
  Legacy: `monitor = desc:…, 3840x2160@60.00, 0x0, 1.5, transform, 0, vrr, 0` ve
  `monitor = DP-2, disable`, `monitor = HDMI-A-1, 1920x1080@60, 0x0, 1, mirror, DP-1`.
- Yansıtma ve `default_monitor` bağlayıcı adı ister: dosya, bağlı küme her değiştiğinde güncel
  adlarla yeniden üretilir (hotplug'da zaten yapılıyor, §8).
- Her üretimden sonra testlerde `Hyprland --verify-config` ile denetlenir (§9).

## 7. IPC sözleşmesi (Ayarlar ↔ shell)

### 7.1 Komutlar

```
displays list                    → JSON (§7.2), ≤ 64 KiB
displays status                  → {"v":1,"gen":N,"pending":null|{"token":"…","remaining_ms":…}}
displays apply <tek satır JSON>  → {"token":"…","confirm":true|false,"deadline_ms":15000} | error: <kod>: <mesaj>
displays confirm <token>         → ok | error: …
displays revert <token>          → ok | error: …
displays identify [<saniye>]     → her monitörde büyük ad etiketi (1..10 sn, varsayılan 3)
```
`gen`: monitör durumunun sürüm sayacı (hotplug, apply, configreloaded'da artar). Ayarlar sayfa
açıkken `status`'u 1 sn'de bir sorgular; `gen` değişince `list` alır (framework IPC'de abonelik
yok; K4).

### 7.2 `list` cevabı

```json
{"v":1,"gen":12,"provider":"lua","pending":null,"primary":"Dell…|ABC123","displays":[{
  "id":"Dell Inc.|DELL U2720Q|ABC123","id_stable":true,"connector":"DP-1",
  "make":"Dell Inc.","model":"DELL U2720Q","serial":"ABC123","description":"Dell Inc. DELL U2720Q ABC123",
  "physical_mm":[600,340],"connected":true,"enabled":true,
  "mode":{"w":3840,"h":2160,"mhz":59997},"pos":[0,0],"scale":1.5,"transform":0,
  "vrr":"off","vrr_capable":null,"mirror_of":null,"logical":[2560,1440],
  "resolutions":[{"w":3840,"h":2160,"rates_mhz":[59997,30000],"preferred":true,
                  "scales":[1,1.25,1.5,1.6,2,2.5,3]}, …]}]}
```
`resolutions[].scales`: o çözünürlük için geçerli (§3.3) ve önerilen ölçekler (1/120 adımındaki
tüm temiz bölenler değil, kullanıcı için anlamlı alt küme: 1, 1.25, 1.333…, 1.5, 1.6, 1.75, 2,
2.25, 2.5, 3 içinden temiz olanlar; Ayarlar "özel" girerse shell doğrular). Sınırlar: ≤ 16
monitör, monitör başına ≤ 64 çözünürlük, ≤ 16 yenileme hızı; dizgiler ≤ 64 karakter.
`vrr_capable`: Hyprland bunu vermiyor → `null` (bilinmiyor; UI yine de seçeneği sunar).

### 7.3 `apply` isteği

```json
{"v":1,"base_gen":12,"primary":"…","displays":[{"id":"…","enabled":true,
 "mode":{"w":3840,"h":2160,"mhz":59997},"pos":[0,0],"scale_120":180,"transform":0,"vrr":"off","mirror_of":null}]}
```
- **Tam istenen durum** (bağlı tüm monitörler), yama değil. `base_gen` güncel değilse `error:
  stale` (Ayarlar yeniden `list` alır).
- Boyut: framework satır sınırı 4096 bayt; anahtarlar kısa tutulursa monitör başına ~230 bayt →
  ~15 monitör sığar. Daha uzunsa `error: too_large` (K3).
- Hata kodları: `stale`, `too_large`, `bad_json`, `unknown_display`, `invalid_mode`,
  `invalid_scale` (öneriyle: `… en yakın: 1.6`), `overlap`, `disconnected_layout`,
  `no_active_display`, `mirror_chain`, `pending_exists`, `locked`, `not_applied`,
  `unavailable` (Hyprland yok/test örneği).
- `serde_json` ile katı tipli ayrıştırma (`deny_unknown_fields`), panic yok.

### 7.4 Güvenlik

- Kurallar yalnızca doğrulanmış modelden üretilir; Lua dizgileri yazdırılabilir ASCII dışını
  `\ddd`, `"` ve `\` kaçışlı yazılır; legacy'de `,` ya da kontrol karakteri içeren açıklama
  kullanılmaz (bağlayıcı adına düşülür).
- Test/ikincil örnek (`NUXUT_SHELL_INSTANCE` varsayılan değil) Hyprland'e **hiçbir monitör
  komutu göndermez** ve `~/.config`'e yazmaz; `displays apply` orada `error: unavailable` döner
  ya da sahte `DisplayControl`'e gider (test kipi).

## 8. Hotplug

- socket2 `monitoraddedv2/monitorremovedv2` → bağlı küme anahtarı hesaplanır.
- Kayıtlı küme varsa: kurallar onaysız uygulanır (daha önce onaylanmıştı), dosya bu küme için
  yeniden üretilir.
- Yoksa: yeni monitör Hyprland'in kendi kuralıyla gelir (`preferred`, `auto`); shell ona
  dokunmaz, yalnızca Ayarlar'a `gen` artışı görünür. (GNOME gibi "yeni monitör en sağa"
  varsayılanını Hyprland zaten `auto` ile yapar.)
- Kapak kapanması (dizüstü) ve adlandırılmış profiller (ev/ofis, el ile geçiş, `displays profile
  …`) **sonraki adım** (K5).

## 9. Test stratejisi ve bu VM'deki bulgular

Kural: kullanıcının Hyprland'inde monitör/çıkış değişikliği yasak (mutlak konumlu fareyi bozar;
kanca engelliyor). Denemeler yalnızca kendi ayrı örneklerimle yapıldı; kullanıcının oturumuna
hiçbir yazma komutu gitmedi (yalnızca `hyprctl monitors all -j` / `status` / `instances` okundu).

| Yöntem | Sonuç (2026-10-10) |
|---|---|
| İç içe Hyprland, **headless sway** içinde | **Çalışmıyor**: aquamarine Wayland arka ucu `xdg_wm_base` v6 istiyor, sway 1.12 v5 veriyor (`invalid version for global xdg_wm_base`), `CBackend::create() failed`. gles2 renderer da aynı. |
| İç içe Hyprland, **headless kwin_wayland** (`--virtual`, `dbus-run-session`, ayrı `XDG_RUNTIME_DIR`, kendi imzası) içinde | **Açılıyor**: `configProvider: lua`, `backend: wayland`; `eval`, `repl`, hata biçimleri, `keyword` reddi, `hl.timer` + iptal, `reload` doğrulandı. **Ama** çıkış 0×0 ve `availableModes` boş: VM'de 3B hızlandırma yok (`failed to create dri2 screen`, `kms_swrast` render düğümünde dumb buffer alamıyor) → kip/ölçek/transform gerçekten uygulanamıyor, sonuç okunamıyor. Not: `XDG_RUNTIME_DIR` kısa olmalı (uzun yolda "Socket2 path is too long"). |
| `Hyprland --verify-config -c dosya.lua` | **Çalışıyor**, compositor gerekmez: üretilen Lua'nın sözdizimi ve alan adları denetlenir. |
| Sahte socket1 (`tests/hyprland_service.rs`'teki `FakeSocket1` kalıbı) | Mevcut; genişletilecek. |

Önerilen katmanlar:
1. **Saf birim testleri** (çoğunluk): ölçek temiz bölen, mantıksal boy/transform, çakışma ve
   bağlılık, normalleştirme, yansıtma zinciri, `apply` JSON ayrıştırma ve sınırlar, Lua/legacy
   üretimi (altın dosyalar + kaçış), durum makinesi (sahte saatle: timeout, Esc, hotplug,
   stale, pending_exists, çöküş sonrası pending dosyası), küme anahtarı/profil seçimi.
2. **Sahte Hyprland** (`FakeHypr`: `DisplayControl` + socket1 sunucusu): `j/monitors all`
   için kaydedilmiş JSON'lar (bu VM'inki dahil; çok monitörlü sentetik), `status` sağlayıcısı,
   eval/keyword kayıt; "Hyprland ölçeği düzeltti / kipi düşürdü" senaryoları → `not_applied` +
   geri dönüş.
3. **`--verify-config`** entegrasyon testi: üretilen her altın Lua dosyası (Hyprland yoksa test
   atlanır).
4. **İç içe Hyprland (kwin içinde)** protokol testi, `flock` altında, betikle: gerçek socket1
   üstünden apply → eval metni kabul ediliyor mu, guard kuruluyor/iptal ediliyor mu, legacy
   kipte (ayrı `.conf`) `keyword monitor` kabul ediliyor mu. Görüntü/sonuç doğrulaması **yok**.
   Not: bu örnekte sanal çıkış eklemek (`output create`) kanca tarafından komut dizgisinden
   engelleniyor; yalıtılmış örnek için istisna kullanıcı kararıdır, istenmedi.
5. **Onay penceresi UI'ı** izole headless sway'de (çoklu çıkış: `test-compositor.sh output add`),
   shell sahte `DisplayControl` ile: tüm monitörlerde görünüyor mu, Esc/Enter/zaman aşımı, kırpılmış
   `grim -o`.
6. **Gerçek donanım** (çok monitör, VRR, kesirli ölçek): bu VM'de yapılamaz; kullanıcının onayıyla
   kendi makinesinde kabul testi listesi olarak ADR'nin devamında tutulur.

## 10. Uygulama sırası

1. `services/displays`: model, doğrulama, ölçek/yerleşim, üretici (Lua+legacy), durum makinesi,
   `DisplayControl` + sahte; testler.
2. `HyprlandClient`: `status` sağlayıcı okuması, `monitors_all` (tam alanlar), `eval`/`keyword
   batch` (yalnızca `displays`'ten tipli çağrı), örnek kapısı.
3. IPC komutları (`src/ipc.rs`) + `display-guard` alt komutu.
4. Onay penceresi + `identify` etiketleri (shell yüzeyi).
5. `displays.toml` (`nuxut-apply` katmanı) + üretilen dosya (`hypr_config` modülü).
6. Hotplug küme hafızası.
7. Sonraki: adlandırılmış profiller, kapak, HDR/renk yönetimi, bitdepth, özel kip.

## 11. Açık kararlar

| # | Soru | Seçenekler | Öneri |
|---|---|---|---|
| K1 | Onay penceresi nerede | (a) tüm etkin monitörler, (b) yalnızca değişmeyen monitör, (c) birincil | **(a)**; değişmeyen monitör olmayabilir (tek monitör), siyah monitör tespiti güvenilmez |
| K2 | Onaysız değişiklikler | (a) her şey onaylı (GNOME), (b) konum/birincil onaysız | **(b)**; konum ekranı karartamaz, sürükle-bırakta her adımda 15 sn soru can sıkar |
| K3 | 4 KiB satır sınırı | (a) kısa şema, ~15 monitör, (b) framework `MAX_LINE`'ı 16 KiB'a çıkarmak, (c) Ayarlar JSON'u `$XDG_RUNTIME_DIR/nuxut/` altına 0600 dosya yazar, yol gönderir | **(a)** şimdilik; 16 monitör sınırı zaten var, gerekirse (b) framework'e |
| K4 | Ayarlar'a canlı güncelleme | (a) `status` yoklaması 1 sn (sayfa açıkken), (b) framework IPC'ye abonelik | **(a)**; ucuz, (b) framework işi |
| K5 | Profiller kapsamı | (a) v1: otomatik küme hafızası, (b) v1: adlandırılmış profiller de | **(a)**; adlandırılmış profiller + kapak sonraki adım |
| K6 | "Birincil" anlamı | (a) `cursor:default_monitor` + shell varsayılanı, (b) + workspace 1 bağlama | **(a)**; (b) `workspaces` ile ayrıca |
| K7 | Doğruluk kaynağı | (a) ayrı `displays.toml`, (b) `shell.toml [displays]` | **(a)**; makine/donanım verisi, küme kayıtları, insan düzenlemesi değil. `shell.toml [displays]` yalnızca `confirm_seconds` gibi davranış ayarlarını tutar |
| K8 | Kullanıcının mevcut monitör kuralları (`monitors.conf`, nwg-displays) | (a) Nuxut kuralları sonra gelip ezer + Ayarlar'da uyarı, (b) ilk açılışta içe aktarıp kullanıcı satırlarına dokunmamak | **(a)** + ilk açılışta mevcut çalışma anı durumunu küme kaydı olarak almak (içe aktarma dosya ayrıştırmadan, `j/monitors all`'dan) |

Doğrulanacaklar (iç içe örnekte görüntü üretilemediği için): aynı çıkış için ikinci
`hl.monitor` eval'ının ilk kuralı değiştirmesi (beklenen: aynı ada kural güncellenir), `desc:`
kuralının bağlayıcı adlı kurala göre önceliği, `mirror` alanında `desc:` kabul edilip edilmediği.

## 12. Bağımlılıklar

- **`hypr_config`:** config sağlayıcısı tespiti (Lua/legacy) ve ana dosya yolu; Nuxut modülünün
  `displays.lua` / `nuxut-displays.conf`'u yüklemesi; canlı uygulamada `reload config-only`
  kuralı; kullanıcı monitör kurallarının taramada `managed_overrides`'a eklenmesi.
- **`nuxut-apply`:** `displays.toml` yazımı için atomik yazım + ayar başına snapshot + geri al API'si;
  `shell.toml [displays] confirm_seconds` şeması.
- **Ayarlar:** Ekranlar sayfası (mantıksal dikdörtgen tuvali, yapışma, kip/ölçek/döndürme/
  VRR/yansıtma/birincil/açma-kapama denetimleri), `list`/`status` yoklaması, `apply` sonrası
  **onay penceresini kendisi göstermez** (shell gösterir; sayfa yalnızca "onay bekleniyor"
  durumunu yansıtır), "Tanımla" düğmesi → `identify`.
- **Framework (gerekirse):** `MAX_LINE` artışı (K3) ya da IPC aboneliği (K4); onay
  penceresi için mevcut `Dialog` yeterli görünüyor.

## 13. Uygulama (2026-10-10) — bağlayıcı IPC şeması

Kod: `src/services/displays/` (`model`, `validate`, `render`, `store`, `protocol`, `guard`,
`manager`), çalışma anı thread'i `src/app/displays.rs` (`nuxut-displays`), IPC ayrıştırma
`src/ipc.rs` (`DisplaysCommand`). Hyprland'e erişim `hypr_config`'ın `HyprControl` trait'i üzerinden
(okuma her zaman; yazma yalnızca `SyncEnv::may_control`). Testler sahte Hyprland (`FakeHypr`:
`j/monitors all` fikstürleri, yazma komutu kaydı, uygulama sonrası durum kuyruğu) ve bellek
dosyalarıyla; üretilen Lua/conf ayrıca `Hyprland --verify-config` ile denetlenir.

### 13.1 Taşıma

Framework IPC argümanları **sekmeyle** ayırır (`nuxut::ipc::send_args`); argüman boşluk
içerebilir, sekme/satır sonu içeremez (serde_json dizgi içinde kaçışlar). Ayarlar:
`send_args(app, ["displays", "apply", <tek satır JSON>])`. Satır sınırı 4096 bayt → JSON ≤ 4000
bayt. Cevap tek satır; hata `error: <kod>: <mesaj>`. İstemci zaman aşımı 2 sn (framework) →
`apply` doğrulaması en çok ~0,9 sn bekler (6 × 150 ms).

### 13.2 Komutlar

| Komut | Cevap |
|---|---|
| `displays list` | `ListJson` (aşağıda) |
| `displays status` | `{"v":1,"gen":N,"provider":"lua"\|"hyprlang"\|null,"available":bool,"pending":null\|{"token":"…","remaining_ms":N,"confirm":true},"last":null\|{"token":"…","result":"…","detail":"…"?}}` |
| `displays apply <JSON>` | `{"token":"<hex>","confirm":bool,"deadline_ms":N}` — `confirm:false` ise değişiklik uygulandı ve kalıcı yazıldı (`deadline_ms` 0) |
| `displays confirm <token>` | `ok` |
| `displays revert <token>` | `ok` |

`last.result`: `applied` (onaysız uygulandı) · `confirmed` · `reverted` · `timeout` · `hotplug` ·
`not_applied`. `available:false` → bu örnek Hyprland'e yazamaz (test örneği/erişim yok); `apply`
`error: unavailable` döner. `gen` monitör durumu her değiştiğinde (okunan durum farklı, apply,
confirm, geri dönüş) artar; Ayarlar `status`'u sayfa açıkken 1 sn'de bir yoklar, `gen`
değişince `list` alır.

`list`:
```json
{"v":1,"gen":12,"provider":"lua","available":true,"pending":null,"primary":"<id>|null",
 "displays":[{"id":"Dell Inc.|DELL U2720Q|ABC123","id_stable":true,"connector":"DP-1",
   "make":"Dell Inc.","model":"DELL U2720Q","serial":"ABC123","description":"Dell Inc. DELL U2720Q ABC123",
   "physical_mm":[600,340],"enabled":true,"mode":{"w":2560,"h":1440,"mhz":59951},"pos":[1920,0],
   "scale":1.0,"scale_120":120,"transform":0,"vrr":"off|on|fullscreen|fullscreen-media","vrr_active":false,
   "vrr_capable":null,"mirror_of":"<id>|null","logical":[2560,1440]|null,"primary":false,
   "resolutions":[{"w":3840,"h":2160,"rates_mhz":[60000,30000],"preferred":true,
                   "scales":[1.0,1.25,1.5,2.0,3.0],"scales_120":[120,150,180,240,360]}]}]}
```
- `id`: eşleme anahtarı. `id_stable:false` (seri yok ya da aynı kimlikten iki tane) iken
  `make|model|serial|<bağlayıcı>` biçimindedir; Ayarlar onu opak dizgi olarak kullanır.
- `mode`/`pos`/`scale`/`transform`/`enabled`/`mirror_of` Hyprland'in **şimdiki** durumudur;
  `vrr` ve `primary` yapılandırılan değerdir (Hyprland güvenilir vermez).
- `resolutions` alana göre azalan; `rates_mhz` azalan; `scales_120` o çözünürlükte temiz bölen
  olan önerilen ölçekler (120,150,160,180,192,210,240,270,300,360,480 alt kümesi). Ayarlar
  ölçeği daima `scale_120` (tam sayı) olarak gönderir.
- `logical`: döndürme dahil mantıksal boy (kapalı ya da ölçek temiz değilse `null`); Ayarlar
  tuvali bununla çizer, kendisi hesaplamaz.

`apply` (katı: bilinmeyen alan `bad_json`):
```json
{"v":1,"base_gen":12,"primary":"<id>"|null,"displays":[{"id":"<id>","enabled":true,
  "mode":{"w":2560,"h":1440,"mhz":59951},"pos":[1920,0],"scale_120":120,"transform":0,
  "vrr":"off","mirror_of":null}]}
```
**Tam durum**: bağlı her monitör tam bir kez (eksik → `missing_display`, fazla/bilinmeyen →
`unknown_display`). Kapalı monitörün `mode/scale` alanları yine gönderilir (denetlenmez).
Yansıtan monitörün `pos`'u yok sayılır (hedefinkine eşitlenir). Shell yerleşimi normalleştirir
(en küçük x,y → 0), birincil yoksa sol-üstü seçer.

Hata kodları: `bad_json`, `too_large`, `stale` (base_gen eski ya da jeton geçersiz/bitmiş),
`unknown_display`, `missing_display`, `invalid_mode`, `invalid_scale` (mesajda `en yakın: <ölçek>`),
`invalid_position`, `overlap`, `disconnected_layout`, `no_active_display`, `mirror_chain`,
`invalid_primary`, `pending_exists`, `not_applied` (Hyprland istenen durumu uygulamadı → shell
otomatik geri döndü), `unavailable`. Mesaj, ilgili monitörün `id`'siyle başlar (`<id>: …`).

### 13.3 Davranış

- Onaysız (K2): yalnızca `pos` ve/veya `primary` değişimi. Diğer her değişiklik `Pending`.
- Süre `shell.toml [displays] confirm_seconds` (5..=60, varsayılan 15; şemada).
- Pending kaydı `$XDG_RUNTIME_DIR/nuxut/displays-pending-<örnek>.json` (jeton, son an, dil,
  önceki yerleşim). Shell açılışında varsa önceki yerleşime dönülür. Lua: aynı `eval` içinde
  `NUXUT_DISPLAY_GUARD = hl.timer(…, timeout = (süre+5)·1000)`; confirm `set_enabled(false)`;
  her yeni eval/geri dönüş önce eski zamanlayıcıyı iptal eder. hyprlang: `dispatch exec <exe>
  display-guard <jeton> <son an> <örnek>` (kayıt hâlâ aynı jetonla duruyorsa `reload`).
- Kapanışta bekleyen değişiklik geri alınır.
- Geri dönüş komutu Hyprland'e ulaşamazsa (ya da reddedilirse) pending kaydı **silinmez**:
  Lua'da zamanlayıcı, hyprlang'da bekçinin `reload`'u, her durumda açılış kurtarması geri
  döner. Lua geri dönüşünde sıra: önce kurallar, **sonra** zamanlayıcı iptali (bir kural hata
  verirse eval durur, zamanlayıcı kurulu kalır).
- Bilinmeyen ya da artık geçersiz küme + hiç etkin, yansıtmayan monitör yok (ör. dizüstü
  ekranı kapalı onaylanmış, harici çıkarılmış): dosya boşaltılır **ve** kapalı/yansıtan
  monitörler `preferred, auto, auto` ile açılır (çalışma anı kuralı dosya boşaltmakla kalkmaz).
- Hotplug kararı `handled_set` ile verilir: `list`/`apply`/doğrulama okumaları küme
  değişimini "tüketmez".
- Açıklama kırpılmış/arındırılmışsa (`description_exact = false`) `desc:` yerine bağlayıcı adı.
- Test örneğinin yazma izni (`may_control`) ayrıca `$XDG_RUNTIME_DIR/hypr`, `hypr/<imza>` ve
  `.socket.sock`'un gerçek yollarının `/run/user` dışında olmasını ister (bağlantıyla atlatma yok).
- **Ürün kararı (2026-10-11):** onay penceresi gelene kadar Ayarlar'daki "Koru"
  düğmesi `displays confirm <token>` gönderir; "Geri al" `displays revert <token>`. Sayfa
  `status.pending.remaining_ms` ile geri sayımı gösterir. Shell penceresi gelince Ayarlar'ın
  düğmeleri kalabilir (ikisi de aynı jetonu kullanır; ilk gelen kazanır, ikincisi `stale`).
- Hotplug: `j/monitors all`'dan hesaplanan küme anahtarı değişince — bekleyen varsa geri dönüş
  (`hotplug`), sonra kümenin kaydı varsa onaysız uygulanır ve üretilen dosya o küme için
  yeniden yazılır; kayıt yoksa (mevcut) dosya boşaltılır, Hyprland/kullanıcı kuralları geçerli.
- K8: ilk açılışta kayıt yoksa çalışma anı durumu yalnızca **bellekte** temel alınır; diske ilk
  onayda yazılır (kullanıcının açıkça bir şey yapmadan Hyprland ayarına dokunulmaz).
- `hypr_config` bağlantısı: üretilen dosya varsa `nuxut.lua` sonuna `pcall(require, "nuxut.displays")`
  (dosya `<hypr>/nuxut/displays.lua`, kendi pcall'ı var), `nuxut.conf` sonuna `source =
  ~/.config/hypr/nuxut-displays.conf` eklenir (`GenerateInput::displays`). Ekranlar kalıcı
  yazınca eşitleme zorla yeniden çalışır.
- `displays.toml` ve üretilen dosya `nuxut-apply` ile (`services::apply::write_derived`: atomik,
  snapshot yok; Nuxut başlığı taşımayan dosyanın üzerine yazılmaz, `externally_managed`).

### 13.4 Henüz yok

Onay penceresi (shell yüzeyi; K1) ve `identify` etiketleri (UI işi, sonraki adım — o gelene dek
onay Ayarlar'ın "Koru" düğmesiyle `displays confirm`; aksi hâlde süre dolunca geri dönülür; §13.3), `locked` (shell'de
kilit ekranı yok), `configreloaded`'da Pending'i "reloaded" kapatma, adlandırılmış profiller,
kapak, gerçek donanım kabul testi.
