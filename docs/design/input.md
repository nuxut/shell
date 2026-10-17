# Sözleşme: girdi ayarları v1 (t120/t121) — onaylı 2026-10-12

Shell ve Ayarlar arasında. Ayarlar yalnızca `shell.toml` yazar ve `nuxut-shell msg input status` okur.
Shell `[input]`'tan Hyprland satırları üretir (hypr_config: .conf ve Lua), canlı uygular.

## İlke: içe aktar, ezme
- `[input]` altındaki HER anahtar isteğe bağlıdır. Dosyada yazılmamış anahtar için nuxut.conf / Lua
  çıktısına HİÇ satır üretilmez → kullanıcının Hyprland ayarındaki değer (ör. `kb_layout = tr`) aynen kalır.
- Ayarlar etkin değeri `msg input status`'tan gösterir; anahtarı yalnızca kullanıcı o satırı değiştirince
  yazar. Düzen listesi etkin değerden başlar (`tr` + eklenen → `["tr","us"]`).
- "Varsayılana dön" = anahtarı dosyadan SİLMEK (Hyprland/kullanıcı değerine geri döner).
- Serde varsayılanı yok sayılır: shell `Option<_>` alanlar kullanır; şema `default` alanı yalnızca
  Ayarlar'ın "varsayılan" göstergesi için Hyprland varsayılanını taşır.

## shell.toml
```toml
[compositor]
sync_input = true          # varsayılan true; false ise [input]'tan hiç satır yok

[input.keyboard]
layouts = [{ layout = "tr", variant = "" }, { layout = "us", variant = "intl" }]   # 1..=4
options = ["grp:alt_shift_toggle", "caps:escape"]   # tam kb_options listesi, 0..=16
repeat_rate = 25           # 1..=100 (tuş/sn)
repeat_delay = 600         # 100..=2000 (ms)
numlock_by_default = false

[input.mouse]
sensitivity = 0.0          # -1.0..=1.0 (ortak hız: input:sensitivity; fare ve dokunmatik)
accel_profile = "adaptive" # "adaptive" | "flat"
natural_scroll = false     # input:natural_scroll (fare)

[input.touchpad]
natural_scroll = false
tap_to_click = true
disable_while_typing = true
scroll_factor = 1.0        # 0.1..=3.0
scroll_method = "2fg"      # "2fg" | "edge" | "on_button_down" | "no_scroll" (input:scroll_method)
```
Doğrulama (shell): ad `[A-Za-z0-9_()+:-]{1,64}` (variant boş olabilir, layout boş olamaz), seçenek `^[a-z0-9_]+:[A-Za-z0-9_()+-]{1,64}$`. Geçersiz değer → o anahtar yok sayılır
+ uyarı (diğerleri uygulanır). Aralık dışı sayı → kırpılmaz, yok sayılır + uyarı.

## Hyprland eşlemesi
| anahtar | Hyprland |
|---|---|
| keyboard.layouts | `input:kb_layout` = layout'lar virgülle, `input:kb_variant` = variant'lar virgülle (ikisi birlikte yazılır) |
| keyboard.options | `input:kb_options` = virgülle (boş liste → `input:kb_options =` boş, yani açıkça temizler) |
| keyboard.repeat_rate / repeat_delay / numlock_by_default | `input:repeat_rate` / `input:repeat_delay` / `input:numlock_by_default` |
| mouse.sensitivity / accel_profile / natural_scroll | `input:sensitivity` / `input:accel_profile` / `input:natural_scroll` |
| touchpad.natural_scroll / tap_to_click / disable_while_typing / scroll_factor | `input:touchpad:natural_scroll` / `input:touchpad:tap-to-click` / `input:touchpad:disable_while_typing` / `input:touchpad:scroll_factor` |
| touchpad.scroll_method | `input:scroll_method` |
Hepsi MANAGED_KEYS'e (yalnızca yazıldığında) girer → mevcut override tespiti (kullanıcı dosyası
nuxut.conf kaynağından SONRA aynı anahtarı yazıyorsa) bu anahtarlar için de çalışır.

## Şema (`nuxut-shell schema`)
`compositor.sync_input` (bool), `input.keyboard.repeat_rate` (int 1..100, default 25, unit "/sn"),
`input.keyboard.repeat_delay` (int 100..2000, default 600, unit "ms"), `input.keyboard.numlock_by_default`
(bool false), `input.mouse.sensitivity` (float -1..1, default 0), `input.mouse.accel_profile`
(choice adaptive|flat, default adaptive), `input.mouse.natural_scroll` (bool false),
`input.touchpad.natural_scroll` (false), `input.touchpad.tap_to_click` (true),
`input.touchpad.disable_while_typing` (true), `input.touchpad.scroll_factor` (float 0.1..3, default 1),
`input.touchpad.scroll_method` (choice 2fg|edge|on_button_down|no_scroll, default 2fg).
`input.keyboard.layouts` ve `input.keyboard.options` liste türü — şemada mevcut liste/dizi türü neyse
(yoksa "custom" türü, Ayarlar özel blokla yazar). Hepsi `since = 2`.

## `msg input status` (salt okunur JSON, sürüm 1)
```json
{ "v": 1,
  "available": true,
  "keyboard": { "layouts": [{"layout":"tr","variant":""}], "options": [],
                "repeat_rate": 70, "repeat_delay": 200, "numlock_by_default": true },
  "mouse":    { "sensitivity": 0.0, "accel_profile": "", "natural_scroll": false },
  "touchpad": { "natural_scroll": true, "tap_to_click": true, "disable_while_typing": true,
                "scroll_factor": 1.0, "scroll_method": "" },
  "managed":  ["input.keyboard.layouts"],
  "overrides": [{"key": "input.keyboard.repeat_rate", "hypr": "input:repeat_rate", "file": "~/.config/hypr/…", "line": 12}] }
```
- Değerler Hyprland `getoption` (IPC) ile ETKİN değerlerdir. `accel_profile`/`scroll_method` Hyprland'de
  boş ("" = libinput varsayılanı) olabilir; olduğu gibi döner.
- `available: false` → Hyprland'e ulaşılamadı; tüm değer alanları `null`.
- `managed`: shell.toml'da yazılı ve geçerli `[input]` anahtarları. `overrides`: managed olup kullanıcı
  dosyasının geçersiz kıldıkları (mevcut hypr sync override mekanizması; dosya yolu `~` kısaltmalı).
- Komut zaman aşımı: Ayarlar 2 sn bekler.

## Canlı uygulama
shell.toml değişince mevcut hypr_config senkronu (yeniden üret + uygula); ayrıca bir şey yok.
Düzen değiştirme kısayolu çakışması: shell keybind hattı denetler; Ayarlar Kısayollar sayfasındaki mevcut
çakışma gösterimini kullanır (v1'de grp seçenekleri kısayol listesine katılmaz — not).

## v2 (bu sürümde YOK)
Cihaz başına hız/ayar (`device {}` blokları), bar düzen göstergesi (kabuk).

## Uygulama kararları (2026-10-13)
- Kaynak: girdi sözleşmesi v1 (bu dosya repodaki kalıcı kopyadır; değişiklik burada yapılır).
- `overrides[].file`/`line` `null` olabilir: Lua'da kaynak sırası taraması yok; shell `getoption`
  değerini ürettiğiyle karşılaştırır, uyuşmazlık yalnızca uygulama oturmuşken override sayılır
  (inceleme). Ayarlar dosyasız notu Hyprland anahtar adıyla gösterir.
- Nuxut dosyası iki kez source edilirse override sınırı SON source (şart).
- Bilinen sınırlar: `device:<ad>:*` görünmez; betikle `hyprctl keyword` yeri `null` override.
