# Oturumu kapat (kabuk → compositor çıkışı, logind yedeği)

2026-10-16 (kullanıcı hatası: "Oturumu kapat siyah ekrana bıraktı"). Kod:
`src/services/logout.rs`, `src/app/session.rs` (`logout`, `on_logout`), `src/services/hyprland.rs`
(`compositor_pid`, `config_provider`, `dispatch_raw`).

## Kök neden

Oturum kartındaki "Oturumu kapat" doğrudan logind `Session.Terminate()` çağırıyordu. Kullanıcının
makinesinde (Arch, tty1 → bash → `start-hyprland`, DM yok, NVIDIA + Intel iki GPU) tty'ye dönüş
siyah ekranda kaldı.

Terminate oturumu compositor'dan ÖNCE söker: logind oturumu durdururken aygıtlarını geri alır
(DRM ana kipi bırakılır, giriş aygıtları `EVIOCREVOKE`), VT'yi devralır ve kapsamdaki bütün
süreçlere aynı anda SIGTERM gönderir (bash dahil; getty tty1'i yeniden açar). Hyprland'in
SIGTERM yolu kendi başına temizdir — 0.56.2 kaynağı: `handleCritSignal` → `stopCompositor()` →
döngü döner, `main` `cleanup()` çağırır ve watchdog'a `end` yazar; `start-hyprland` da SIGTERM'de
`forceQuit()` ile çocuğu bekler, güvenli kipte yeniden başlatmaz (yeniden başlatma yalnız `end`
gelmeden ölünce: "Hyprland exit not-cleanly, restarting"). Ama temizlik, aygıtı artık elinde
olmayan bir oturumda koşar: CRTC/VT durumu geri verilemez, sürücü (özellikle NVIDIA) çevirme
beklerken takılabilir; kapsam durdurma zaman aşımına (varsayılan 90 sn) dek SIGKILL gelmez, ekran
grafik kipte siyah kalır. Kesin takılma noktası kullanıcının makinesinde gözlenmedi (bu VM'de
NVIDIA yok); düzeltme bu yarıştan bağımsız: compositor oturum hâlâ etkinken kendi isteğiyle çıkar.

`dispatch exit` ile Hyprland aygıtları elindeyken temizlenir, `end` yazar, `start-hyprland`
"exit cleanly" ile döner; kullanıcı tty1'deki bash'e iner (giriş betiği `exec start-hyprland`
kullanıyorsa oturum tamamen kapanır, getty giriş istemi gelir).

## Tasarım

Sıra: **compositor'a çıkmasını söyle → olmazsa logind Terminate.**

1. **uwsm altındaysa `uwsm stop`.** Tespit: Hyprland örnek dizinindeki `hyprland.lock`'un ilk
   satırı compositor PID'idir; `/proc/<pid>/cgroup` `wayland-wm@*.service` içeriyorsa uwsm
   oturumu. uwsm graphical-session birimlerini sırayla durdurur; doğrudan `exit` uwsm'in
   temizliğini atlardı. `uwsm stop` 5 sn içinde hatayla biterse sonraki yol denenir.
2. **Hyprland `dispatch exit`.** Ayar sağlayıcısı `j/status` → `configProvider`'dan okunur; Lua
   kipinde `dispatch hl.dsp.exit()` (Hyprland 0.56.2 `HyprCtl.cpp` `dispatchRequest`: Lua'da
   girdi `return hl.dispatch(<girdi>)` olarak değerlendirilir; `hl.dsp.exit` argümansız,
   `LuaBindingsDispatchers.cpp`). Cevap `ok` değilse hata.
3. **Yedek: logind `Session.Terminate()`** — hiçbir yol işlemezse hemen; istek kabul edildiyse
   ama kabuk 10 sn (`FALLBACK_AFTER`) sonra hâlâ yaşıyorsa (compositor çıkmadı) yine.

IO ana thread dışında (`nuxut-logout` thread'i); sonuç `ShellMessage::Logout` ile döner.
Hyprland soketi yok/yanıtsızsa `HyprlandClient` zaman aşımıyla hata verir (asılma yok).

Test örneği (`NUXUT_SHELL_INSTANCE`) oturumu kapatmaz: kullanıcının compositor'ını kapatmasın.

## Sınırlar

- Hyprland dışındaki compositor'larda (Nuxut yalnızca Hyprland hedefler) doğrudan Terminate.
- Gerçek `dispatch exit` canlı test edilmedi (kullanıcının oturumunu kapatırdı; test compositor'ı
  sway). Sahte Hyprland soketiyle sözleşme testi: `tests/it/logout.rs` (hyprlang, Lua, ret,
  ulaşılamayan soket). Gerçek makinede kullanıcı doğrular.
- `dispatch_dpms` (`dispatch dpms on|off`) Lua kipinde çalışmaz (`hl.dsp.dpms` olmalı); ayrı iş.
