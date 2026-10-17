# Hyprland 0.56.2 başvuru notları (Nuxut için)

Durum: 2026-10-15. Sabitlenen sürüm: **Hyprland 0.56.2** (`hyprctl version`: commit
`efb50993780079460b0cbed1363e2166a2de1d9f`, etiket `v0.56.2`). Bu belge üçüncü taraf "skill" yerine
geçer (kullanıcı kararı, 2026-10-14); yeni sürümde maddeler yeniden doğrulanır.

Kaynaklar ve işaretler:
- **[kaynak]** — `github.com/hyprwm/Hyprland` etiketi `v0.56.2`, salt okunur sığ klon; yol:satır o
  etikette.
- **[örnek]** — kurulu paket: `/usr/share/hypr/hyprland.lua` (örnek ayar),
  `/usr/share/hypr/stubs/hl.meta.lua` (Lua API taslakları, üretilmiş). Pakette `hyprland.conf` örneği
  ve man sayfası yok (`man Hyprland` boş).
- **[wiki]** — `wiki.hypr.land` (2026-08-26 sürümü; "0.55'ten beri hyprlang kullanımdan kalktı,
  Lua"). Yalnızca okundu.
- **[canlı]** — çalışan oturumda salt okuma (`hyprctl -j status`, `getoption`); komut gönderilmedi.
- **[doğrulama]** — `Hyprland --verify-config -c <dosya>` ile üretilen dosyalar (aşağıda §8).
- **doğrulanmadı** — kaynakta/wikide bulunamayan ya da denenmeyen iddia.

## 1. Ayar dili ve dosya seçimi

- `hyprland.lua` varsa `hyprland.conf` yok sayılır; uzantı `.lua` → Lua yöneticisi, değilse hyprlang
  **[kaynak]** (ADR-0003 Bulgu 1). Çalışma anında: `hyprctl -j status` → `configProvider`
  **[canlı]**: bu VM'de `"hyprlang"`.
- hyprlang'da `source = yol` dosyayı **olduğu yerde** açar; değer atamaları sıralıdır, sonra yazan
  kazanır. Kurallar (window/layer rule) eklenir; bind'ler eklenir (aynı tuş iki bind = ikisi de
  çalışır). `unbind = MOD, tuş` önceki bind'i siler **[kaynak]** `legacy/ConfigManager.cpp`
  `handleUnbind`.
- Lua'da `require("a.b")` → `<ayar dizini>/a/b.lua`; `pcall(require, …)` bulunamayan modülde ayarı
  durdurmaz **[kaynak]** (ADR-0003 Bulgu 2). `hl.*` çağrılarındaki hatalar (bilinmeyen alan, tuş)
  Lua istisnası değil, `configerrors`'a yazılır (`Internal::configError`); `pcall` onları yutmaz
  **[doğrulama]**.

## 2. Bind'ler

### 2.1 hyprlang bayrakları — `bind[bayraklar] = MOD, tuş, dispatcher, arg`

`legacy/ConfigManager.cpp:1516` **[kaynak]**: `l` kilitte de, `r` bırakınca, `e` tekrar, `m` fare
(`bindm = MOD, mouse:272, movewindow`), `n` olayı uygulamaya da geçir, `a` otomatik tüketme, `t`
şeffaf (gölgelenmez), `i` değiştiricileri yok say, `s` çok tuşlu (`&`), `o` uzun basış, `d`
açıklama (`bindd = MOD, tuş, açıklama, dispatcher, arg`), `p` kısayol engelleyiciyi aşar, `c` tıklama,
`g` sürükleme, `u` alt eşlemden bağımsız, `k` aygıta özel, `x` giriş yakalamada da. `e` ile `r`/`o`
birlikte olmaz; `m` başka bayrakla olmaz.

Değiştirici alanı **içerme** ile aranır (`SUPER_SHIFT`, `SUPER SHIFT`, `SUPERSHIFT` aynı).

### 2.2 Lua — `hl.bind("MOD + MOD + tuş", dsp, { … })`

`config/lua/bindings/LuaBindingsToplevel.cpp:58-131,132-256` **[kaynak]**:
- Dize `+` ile bölünür; parçalar **tam eşleşmeyle** değiştirici mi diye bakılır (`SUPER|WIN|LOGO|MOD4|META`,
  `SHIFT`, `CTRL|CONTROL`, `ALT|MOD1`, `CAPS`, `MOD2/3/5`); değiştiriciler önce gelmeli. `Super_L`
  değiştirici sayılmaz, tuştur.
- `code:<n>` tuş kodu; `mouse:<n>`, `mouse_up/down/left/right`, `switch:` özel adlar; diğerleri
  `xkb_keysym_from_name(…, CASE_INSENSITIVE)`.
- Seçenekler: `repeating, locked, release, non_consuming, auto_consuming, transparent, ignore_mods,
  dont_inhibit, long_press, submap_universal, click, drag, description|desc, device{inclusive,list},
  allow_input_capture`. `mouse` seçeneği **okunmaz** (örnek ayardaki `{ mouse = true }` etkisizdir);
  fare sürüklemesini `hl.dsp.window.drag()` / `resize()` kendisi yönetir (`releasePending`).
- `hyprctl -j binds`: Lua bind'inde `dispatcher = "__lua"`; `code:` tuşlu Lua bind'inde `key` boş ve
  `keycode` 0 gelir (kod `sMkKeys`'te) — dışarıdan görülemez **[kaynak]** `HyprCtl.cpp:1026-1085`.

### 2.3 Eşleştirme ve klavye düzeni

`KeybindManager.cpp:289-334,348-470,602-850` **[kaynak]**:
- Keysym bind'i, **ilk düzenin değiştiricisiz (seviye 0)** sembolüyle eşlenir (ayrı
  `m_xkbTranslationState`; `input:resolve_binds_by_sym = true` iken etkin düzen). Sonuç: Türkçe Q'da
  `=` Shift ister → `SUPER+equal` çalışmaz; AZERTY'de rakam satırı seviye 0'da `ampersand…` → `SUPER+1`
  çalışmaz. Wiki de "tuş değiştiricisiz erişilebilir olmalı" der **[wiki]**.
- `code:<n>` (evdev+8) düzenden bağımsızdır: rakam satırı `code:10`…`code:19` (1…9, 0).
- Harf keysym'i tuşun üstündeki harfle eşleşir (AZERTY'de `q` A konumundadır; Türkçe Q'da `i`
  "İ" tuşudur, "I" tuşu `idotless`).

### 2.4 Tek değiştiriciye dokunma (`SUPER` → başlatıcı)

Wiki örneği: `hl.bind("SUPER + SUPER_L", hl.dsp.exec_cmd("…"), { release = true })` **[wiki]**;
hyprlang: `bindr = SUPER, Super_L, exec, …`. Davranış **[kaynak]** `KeybindManager.cpp`:
- Super_L basılırken maske henüz SUPER'i içermez → bind eşleşmez, olay geçer.
- Bırakışta maske SUPER'i içerir; `modmaskAtPressTime` farkı değiştiricinin kendisi için özel
  durumdur (`keycodeToModifier`, 736-746) → bind çalışır.
- **Araya başka girdi girerse iptal:** başka bir bind tetiklenince, başka tuş bırakılınca, bağlı bir
  fare düğmesi/tekerleği kullanılınca `shadowKeybinds()` basılı Super_L yüzünden bind'i gölgeler
  (`shadowed = true`, 855-899); gölgeli bind bırakışta atlanır.
- **Bilinen boşluk (kaynaktan çıkarım, denenmedi):** SUPER basılıyken **bağlı olmayan** bir tuşa basıp
  Super'i o tuştan **önce** bırakmak gölgeleme yapmaz (basışta `shadowKeybinds` yalnız bastırılan
  olayda çağrılır) → başlatıcı açılır. Bağsız tuşta Super'i önce bırakmak nadirdir.
- Super_R için ayrı bind gerekir (`Super_R`).

### 2.5 Fare ve tekerlek

`bindm = SUPER, mouse:272, movewindow` / `mouse:273, resizewindow` **[örnek][kaynak]**; Lua
`hl.dsp.window.drag()` / `hl.dsp.window.resize()` (argümansız: fare kipi). Tekerlek: `mouse_down`
(aşağı), `mouse_up`; `binds:scroll_event_delay` (vars. 300 ms) arayla.

## 3. Dispatcher'lar (kullanılanlar)

hyprlang adları `KeybindManager.cpp:42-113`, çevirisi `legacy/DispatcherTranslator.cpp`; Lua
`config/lua/bindings/LuaBindingsDispatchers.cpp:1340-1417` **[kaynak]**.

| İş | hyprlang | Lua |
|---|---|---|
| Pencereyi kapat | `killactive` | `hl.dsp.window.close()` |
| Tam ekran / ekranı kapla | `fullscreen, 0` / `fullscreen, 1` (ikinci kelime `toggle|set|unset`) | `hl.dsp.window.fullscreen({ mode = "fullscreen"|"maximized", action })` |
| Yüzen | `togglefloating` | `hl.dsp.window.float({ action = "toggle" })` (vars. toggle) |
| Sabitle | `pin` | `hl.dsp.window.pin()` |
| Odak | `movefocus, l|r|u|d` | `hl.dsp.focus({ direction = "left" … })` |
| Taşı | `movewindow, l` | `hl.dsp.window.move({ direction = … })` |
| Boyut (göreli) | `resizeactive, 40 0` | `hl.dsp.window.resize({ x, y, relative = true })` |
| Çalışma alanı | `workspace, 3` / `r+1` / `e+1` / `previous` | `hl.dsp.focus({ workspace = 3 | "r+1" … })` |
| Pencereyi alana | `movetoworkspace, 3`, `movetoworkspacesilent, …` | `hl.dsp.window.move({ workspace, follow = false })` |
| Özel alan | `togglespecialworkspace, ad` | `hl.dsp.workspace.toggle_special("ad")` |
| Pencere döngüsü | `cyclenext` / `cyclenext, prev` | `hl.dsp.window.cycle_next({ next = false })` |
| dwindle böl | `layoutmsg, togglesplit` | `hl.dsp.layout("togglesplit")` |
| Komut | `exec, …` | `hl.dsp.exec_cmd("…")` |

`r±n`: kimliğe göre göreli (boş alanlar dahil); `e±n`: açık alanlar; `previous`: son alan.

## 4. Pencere ve katman kuralları (0.53+ sözdizimi, 0.56'da geçerli)

hyprlang **[kaynak]** `legacy/ConfigManager.cpp:2016-2090`: tek satırda virgülle ayrık alanlar;
`match:<özellik> <değer>` eşleşme, `<etki> <değer>` etki. Değerde **virgül olamaz** (alan
ayırıcısı). Eski `windowrulev2` / `layerrulev2` de kayıtlı (geriye uyum).

```
windowrule = match:class ^(nuxut-settings)$, float on, center on, size 1040 720
layerrule  = match:namespace ^(nuxut)$, no_anim on, blur on, ignore_alpha 0.47
```

Eşleşme özellikleri `desktop/rule/Rule.cpp:27-44`: `class, title, initial_class, initial_title,
float, tag, xwayland, fullscreen, pin, focus, group, modal, fullscreen_state_internal,
fullscreen_state_client, workspace, content, xdg_tag, namespace`. `content`: `none|photo|video|game`
(ya da sayı; `ContentType.cpp:9`).

Pencere etkileri `WindowRuleEffectContainer.cpp:13-71`: `float, tile, fullscreen, maximize,
fullscreen_state, move, size, center, pseudo, monitor, workspace, no_initial_focus, pin, group,
suppress_event, content, no_close_for, scrolling_width, rounding, rounding_power, persistent_size,
animation, border_color, idle_inhibit (none|always|focus|fullscreen), opacity, tag, max_size,
min_size, border_size, allows_input, dim_around, decorate, focus_on_activate, keep_aspect_ratio,
nearest_neighbor, no_anim, no_blur, no_dim, no_focus, no_follow_mouse, no_max_size, no_shadow,
no_shortcuts_inhibit, opaque, force_rgbx, sync_fullscreen, immediate, xray, render_unfocused,
no_screen_share, no_vrr, no_auto_hdr, tonemap, scroll_mouse, scroll_touchpad, stay_focused,
confine_pointer`. `move`/`size` değerleri muParser ifadesi: `monitor_w`, `monitor_h`, `window_w`,
`window_h`, `window_x`, `window_y`, `cursor_x`, `cursor_y` (`Window.cpp:2000-2016`); iki ifade
boşlukla ayrılır, ifade içinde boşluk olmaz.

Katman etkileri `LayerRuleEffectContainer.cpp:13-24`: `no_anim, blur, blur_popups, ignore_alpha
(0..1), dim_around, xray, animation, order, above_lock (0..2), no_screen_share`.

Lua **[kaynak]** `LuaBindingsConfigRules.cpp:1167-1300`: `hl.window_rule({ name, enabled, match =
{ class = "…", modal = true }, float = true, size = "1040 720" | {w, h}, … })`; aynı `name` yeniden
yüklemede kuralı günceller (`m_luaWindowRules`). `hl.layer_rule` aynı biçim.

Otomatik yüzen pencereler **[kaynak]** `XWaylandManager.cpp:110-147`: Wayland'de xdg üst penceresi
olan ya da en küçük = en büyük boyutlu; X11'de dialog/splash/… türleri, kipli, geçici, sabit boyut.

## 5. Jestler

hyprlang `gesture = <parmak>, <yön>, [mod:…,] [scale:…,] <eylem>[, arg]` **[kaynak]**
`legacy/ConfigManager.cpp:1931-2014`: eylemler `workspace, resize, move, special, close, float,
fullscreen, cursorZoom, scrollMove, dispatcher, unset`. Lua `hl.gesture({ fingers, direction,
action, mods, scale, … })` **[örnek][kaynak]**. Örnek ayar: 3 parmak yatay → `workspace` **[örnek]**.
`gestures:workspace_swipe_*` ayarları sürüklemenin eşiğini belirler.

## 6. Animasyon ağacı

`config/shared/animation/AnimationTree.cpp:15-66` **[kaynak]**:

```
global ─┬ windows ─┬ windowsIn / windowsOut / windowsMove
        ├ layers ──┬ layersIn / layersOut
        ├ fade ────┬ fadeIn / fadeOut / fadeSwitch / fadeShadow / fadeGlow / fadeDim / fadeDpms
        │          ├ fadeLayers ─ fadeLayersIn / fadeLayersOut
        │          └ fadePopups ─ fadePopupsIn / fadePopupsOut
        ├ border, borderangle, shadowangle, glowangle (açılar varsayılan kapalı)
        ├ workspaces ─┬ workspacesIn / workspacesOut
        │             └ specialWorkspace ─ specialWorkspaceIn / Out
        ├ zoomFactor, monitorAdded
```

- hyprlang `animation = yaprak, 0|1, hız, eğri[, stil]`, `bezier = ad, x1, y1, x2, y2`; hız
  desisaniye (8 = 800 ms) **[kaynak]** `handleAnimation`. Lua `hl.curve(ad, { type = "bezier",
  points = {{x1,y1},{x2,y2}} } | { type = "spring", mass, stiffness, dampening })`,
  `hl.animation({ leaf, enabled, speed, bezier | spring, style })` **[örnek]**.
- Stil denetimi `AnimationManager.cpp:388-458`: pencere `slide [yön]`, `popin [n%]`, `gnome(d)`;
  çalışma alanı `slide`, `slidevert`, `fade`, `slidefade[vert] n%`; katman `fade`, `slide [yön]`,
  `popin n%`.
- **Pencere varsayılanı tehlikeli:** stil verilmezse `applyPopin(…, 0%)` — pencere 5 px'ten büyür
  ("patlama") **[kaynak]** `WindowAnimationController.cpp:143-149`. Stil her zaman açıkça yazılmalı.
- Çalışma alanı kaydırması yön duyarlı: hedef kimlik büyükse sola kayar
  (`Monitor.cpp:1471`) **[kaynak]**.

## 7. Yerleşim ve davranış anahtarları (kullanılanlar)

Varsayılanlar `config/values/ConfigValues.cpp` **[kaynak]**:

| Anahtar | Varsayılan | Not |
|---|---|---|
| `general:resize_on_border` | false | kenardan sürükleyerek boyut |
| `general:snap:enabled` | false | yüzenler yapışır |
| `input:follow_mouse` | 1 | 0 yok, 1 odak imleci izler, 2 tıklayınca odak (imleç ayrı), 3 tamamen ayrı |
| `dwindle:preserve_split` | false | |
| `dwindle:force_split` | 0 | 0 imleç, 1 sol/üst, 2 sağ/alt |
| `misc:disable_hyprland_logo` / `disable_splash_rendering` | false | |
| `misc:force_default_wallpaper` | -1 | 0: maskot duvar kağıdı yok |
| `misc:focus_on_activate` | false | etkinleştirme isteği odak alsın mı |
| `misc:mouse_move_enables_dpms` / `key_press_enables_dpms` | false | |
| `binds:hide_special_on_workspace_change` | false | |
| `ecosystem:no_update_news` | false | |
| `debug:vfr` | true | "hata ayıklama dışında kapatmayın"; **`misc:vfr` 0.56'da yok** |
| `cursor:no_hardware_cursors` | — | Nuxut dokunmaz (VM/sürücüye bağlı) |

## 8. Yalnız doğrulama kipi

`Hyprland --verify-config [-c dosya]` **[kaynak]** `main.cpp:160,263-292`, `Compositor.cpp:186-188,
270-279`: `CCompositor(onlyConfig)` kurucusu hemen döner (imza, `$XDG_RUNTIME_DIR/hypr/<imza>`,
soket yok); `initServer` yalnızca bind yöneticisi + ayar yöneticisini kurar, ayarı ayrıştırır,
"======== Config parsing result" yazar. Compositor, Wayland soketi, IPC **başlatılmaz**; `exec-once`
/ `hl.on("hyprland.start")` çalışmaz. Nuxut'un ürettiği dosyalar bu kiple denetlendi: hyprlang ve
Lua ikisi de "config ok"; bilinçli bozulan alan (`no_focuss`, `qqq` tuşu) hata olarak raporlandı
**[doğrulama]** (ortam: `env -i`, ayrı `XDG_RUNTIME_DIR`, kullanıcı oturumuna dokunulmadı).

## 9. Doğrulanmayanlar

- `Picture-in-Picture` pencere başlıkları tarayıcıya ve dile göre değişebilir (Firefox İngilizce
  "Picture-in-Picture"; Chromium ve yerelleştirilmiş başlıklar denenmedi).
- `match:content` yalnızca `wp_content_type_v1` bildiren istemcilerde çalışır; hangi oynatıcıların
  bildirdiği denenmedi (tarayıcı/oynatıcı oynatırken zaten kendisi idle-inhibit ister).
- §2.4'teki "bağsız tuş + Super önce bırakılır" boşluğu kaynaktan çıkarımdır, canlı denenmedi
  (kullanıcının oturumuna sentetik girdi gönderilmez).
- `follow_mouse = 2` ile kabuk katmanlarının (OnDemand klavye) etkileşimi canlı denenmedi.
