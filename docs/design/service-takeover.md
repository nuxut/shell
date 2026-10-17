# Çakışan bar ve bildirim servislerinin devralınması

2026-10-16 (kullanıcı ilkesi "Nuxut yapabiliyorsa varsayılan Nuxut": başka aracın görevini devral,
yedekle, tek komutla geri al). Kod: `dist/takeover/conflicting-services.sh`; çağıran `install.sh`
("Kontroller", `--dry-run`, `--uninstall`). Test: `tests/it/service_takeover.rs`
(`tests/fixtures/service_takeover.sh`, sahte systemctl/pgrep/pkill).

## Sorun

Kurulum dunst, mako, swaync, waybar, quickshell (`qs`), ags ve eww çalışıyorsa yalnız uyarıyordu.
Bildirim servisi `org.freedesktop.Notifications` adını tutar (Nuxut bildirim merkezi boş kalır);
bar/kabuk ikinci bir bar çizer. Kullanıcı hangisinin nereden başladığını (systemd birimi, D-Bus
etkinleştirmesi, Hyprland `exec-once`) bulup kapatmak zorundaydı.

## Davranış

`install.sh` (kurulumda, `--keep-services` yoksa) `conflicting-services.sh apply` çalıştırır:

1. **Önce kayıt.** Değiştirilecek her birim ve süreç `$XDG_STATE_HOME/nuxut/takeover/services`'e
   yazılır (birimin önceki `UnitFileState` ve `ActiveState`'i, maskeledik mi; durdurulan süreç
   adı). Kayıt geçici dosya + yeniden adlandırmayla yazılır; ilk kayıt kazanır, ikinci kurulum
   özgün durumu ezmez. Taze kurulum (`--reset-config`) bu dosyayı yedeğe taşımaz (izin listesi dışı).
2. **systemd kullanıcı birimi.** Bar: etkinse ya da çalışıyorsa `disable`. Bildirim servisi:
   maskeli değilse her zaman `disable` + `mask` — statik birimler de D-Bus etkinleştirmesiyle
   (`SystemdService=`) başlar; kabuk adı almadan önce gelen ilk bildirim onları yeniden kaldırıp
   adı kapardı. Kullanıcının kendi birim dosyası (`~/.config/systemd/user`) maskelenemez; yalnız
   devre dışı bırakılır.
3. **Bu oturumda durdurma yalnız Nuxut kabuğu çalışacaksa** (`apply --now`: taze kurulum kabuğu
   kapattı ya da yeniden başlatma yapılacak ve kabuk zaten çalışıyor). Birimler `--now` ile durur,
   systemd dışında çalışan süreçler (ör. `exec-once`) yalnız bu kullanıcının ve tam adla
   (`pkill -x -u`) SIGTERM alır. Kabuk çalışmayacaksa (ilk kurulum, `--no-restart`, TTY) hiçbir
   şey durmaz: kullanıcı oturumun ortasında barsız/bildirimsiz kalmaz; değişiklik sonraki girişte
   etkin olur.
4. **Hyprland `exec-once` satırlarına dokunulmaz.** Ana ayarın devralınması ADR-0007'nin işi
   (eski ana dosya yedeklenip yerine Nuxut'unki konunca eski `exec-once` satırları taşınmaz).
   Satır hâlâ bir `.conf`/`.lua` dosyasında duruyorsa dosya:satır yazdırılır.

`--dry-run` `plan` çalıştırır (hiçbir şey yazmaz/değiştirmez). `--uninstall` `restore`:
maskeyi kaldırır, önceden etkin olanı yeniden etkinleştirir, çalışanı başlatır, kaydı siler;
`exec-once` süreçleri sonraki girişte kendi satırlarından döner.

## Sınırlar

- Kapsam sabit liste (yukarıdaki yedi ad). Başka bir bildirim servisi (ör. `xfce4-notifyd`)
  devralınmaz; kabuk adı alamazsa günlükte görünür.
- systemd dışında başlatılan bir süreç restore'da hemen geri başlatılmaz (özgün komut satırı
  bilinmez); sonraki girişte kendi satırından başlar.
