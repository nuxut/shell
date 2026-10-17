# `nuxut-shell doctor`: tek komutla durum raporu

İstek (2026-10-16): Kullanıcıdan ayrı ayrı istenen tanı komutlarını (`msg
audio-diagnose`, `msg hypr status`, `journalctl`…) tek bir okunur rapora toplar.

**Kullanıcıya önerilecek tek komut:**

```sh
nuxut-shell doctor
```

(`~/.local/bin` PATH'te değilse: `~/.local/bin/nuxut-shell doctor`. Makine çıktısı için
`--json`. `nuxut-shell msg doctor` de aynı raporu verir.)

## Kurallar

- **Yalnız okur.** Hiçbir dosyaya yazmaz, hiçbir ayarı/servisi değiştirmez, alt süreç
  başlatmaz. Canlı bilgi için çalışan kabuğa tek salt-okur istek (`msg doctor live`),
  logind'e salt-okur `GetSessionByPID`, systemd kullanıcı yöneticisine salt-okur
  `Manager.Environment` özellik okuması yapar.
- **Kişisel veri girmez.** Dosya içeriği, ağ/cihaz adları, pencere başlıkları, ortam değerleri
  rapora yazılmaz; yalnızca yollar (ev dizini `~` ile), sayılar, sabit durum kimlikleri ve
  ortam değişkeni *adları*.
- **Kabuk çalışmasa da çalışır.** Rapor istemci süreçte üretilir: terminalin PATH'i, kurulu
  dosyalar, kullanıcı ayarları buradan okunur; kabuk yoksa canlı bölümler "kabuk çalışmıyor"
  der.
- Metin katalogdan (`shell-doctor-*`, kabuğun dili). JSON alan adları ve `id`'ler sabit,
  İngilizce; her denetimde çevrilmiş `text`/`hint` de bulunur.
- Çıkış kodu: herhangi bir ✗ varsa 1, yoksa 0.
- Her ⚠/✗ satırının altında tek satırlık `Çözüm:` önerisi vardır (test: `every_problem_has_a_fix_line`).
- **Yetki yükseltme ya da sahiplik komutu önerilmez** (`sudo`, `chown`, `doas`, `pkexec`; 2026-10-16):
  başka kullanıcıya ait görünüm dosyası root sahipli bir temaya sembolik bağ olabilir, `chown` bağı
  izleyip sistem dosyasının sahibini değiştirir. Sahipsiz dosyanın çözümü yedekli devralmadır;
  eksik paket yalnız adlarıyla önerilir (test: `never_suggests_sudo_or_chown`).

## Bölümler ve denetimler

| Bölüm | Denetim (`id`) | Kaynak | ✗ / ⚠ |
|---|---|---|---|
| `install` | `path` | **raporun ilk satırı.** Terminalin `PATH`'i ikili dizinini içeriyor mu; öneri `$SHELL`'in adına göre (yalnız `fish/bash/zsh/other` kimliği rapora girer) | yok ⚠ → fish: `fish_add_path <dizin>` (yeni terminallerde hemen); bash/zsh/diğer: `install.sh --fix-path` + oturumu kapatıp açma |
| | `session_path` | **etkili PATH** (2026-10-16, gerçek makine bulgusu: environment.d dosyası vardı ama Hyprland ve systemd ortamında `~/.local/bin` yoktu). Dosyanın varlığı yetmez: systemd kullanıcı yöneticisinin `Environment`'ı, Hyprland sürecinin ve çalışan kabuğun `/proc/<pid>/environ` PATH'i denetlenir; değer rapora girmez, yalnız ortam adı | görmeyen ortam ⚠ → dosya yoksa `install.sh --fix-path` + yeniden giriş; dosya varsa tam yeniden giriş; yalnız Hyprland/kabuk görmüyorsa uwsm + `nuxut-shell.service` |
| | `binaries` | çalışan ikilinin dizininde `install.sh` `BINS`: `nuxut-shell nuxut-lock nuxut-polkit nuxut-settings nuxut-calendard` (2026-10-16 gerçek vaka: `nuxut-lock` kurulmamıştı). Liste install.sh ile `tests/it/install_bins.rs` `doctor_checks_exactly_what_install_installs` testinde eşlenir | eksik ✗ |
| | `helpers` | `../lib/nuxut/{nuxut-image-worker,xdg-desktop-portal-nuxut,nuxut-sharepicker,nuxut-filechooser}` (install.sh `HELPERS` + `PORTAL_BIN PICKER_BIN CHOOSER_BIN`) | portal/seçici eksik ✗ (kurulum zorunlu sayar); yalnız görsel işçisi eksik ⚠ |
| | `version` | sürüm + derleme commit'i (`NUXUT_BUILD_COMMIT`, install.sh verir) | bilgi |
| `shell` | `running` | tek örnek IPC'si (`msg doctor live`) | yanıt yok ✗ |
| | `service` | `/proc/<pid>/cgroup`: `nuxut-shell.service` / oturum scope'u / uygulama scope'u | uygulama scope'u ⚠ |
| | `instances` | aynı ikiliyi çalıştıran süreç sayısı | >1 ⚠ |
| | `session` | logind `GetSessionByPID(pid)`; olmazsa compositor'ın PID'i (`hyprland.lock`), o da olmazsa `session/auto` (kabuğun kendi çözüm sırası) | auto yedeği ⚠, yok ✗ |
| | `competitors` | `/proc/*/comm`: dunst, mako, swaync, waybar, quickshell/qs, ags, eww (install.sh "Kontroller" listesi; yalnız ad) — kabuk çalışmasa da | ⚠ |
| `hyprland` | `source` | canlı `hypr status` (`present/added` ✓; `off/disabled_by_user/removed_by_user` ⚠; diğerleri ✗) | |
| | `conflicts` | çakışan kısayollar: eylem, tuş, `dosya:satır` | ⚠ |
| | `theme_pack` | kullanıcının Hyprland ayarı ve bir düzey `source` dosyaları: bilinen tema paketi izleri (end4/illogical-impulse, caelestia, ml4w, HyDE, JaKooLit, omarchy) — yalnız paket adı ve `dosya:satır` | ⚠ |
| `look` | `writer` | canlı `look status`: yazıcı | kapalı bilgi |
| | `conflicts` | ortamı ezen değişkenlerin *adları* ve kaynağı | ⚠ |
| | `not_owned` | `last_apply.external` içinde nedeni `not_owned` olan dosyalar (başka kullanıcıya, ör. root, ait; 2026-10-16 gerçek vaka) — ayrı satır | ⚠ → "Nuxut bu dosyayı devralabilir: Ayarlar › Görünüm (ya da `nuxut-shell look apply`); eski hâli yedeklenir" (devralma varsayılan, look 98ff626) |
| | `external` | başkasının yönettiği diğer dosyalar (`last_apply.external`: ayar, yol, neden; look status v5) | ⚠ |
| `audio` | `pipewire` / `sink` / `source` | ses servisinin özeti (`AudioSummary`): arka uç, varsayılan çıkış/giriş, seviye yolu (`route` ✓, `props` ✓, `await_routes` ⚠, `card_props` ⚠, `none` ✗) | |
| `portal` | `files` | `nuxut.portal`, D-Bus servisi, birim, ikili | eksik ⚠ |
| | `desktop` | terminalde `XDG_CURRENT_DESKTOP` `Nuxut` içeriyor mu | ⚠ |
| | `portals_conf` | `nuxut-portals.conf` var mı, Nuxut yönetiyor mu (ilk satır işareti), `FileChooser` Nuxut mu | yok ⚠; dosya seçici başka arka uçta bilgi (kullanıcının açık seçimi) → `Çözüm:` kaynak dosyadaki satırı silip `install.sh --write-portals-conf` |
| | `sharepicker` | `hypr/xdph.conf`'ta `# >>> nuxut-sharepicker` bloğu | ⚠ |
| `lock` | `binary` | `nuxut-lock` kabuk ikilisinin yanında, çalıştırılabilir | ✗ |
| | `listener` | `shell.toml [lock] listen_logind` (yoksa `true`); canlı `lock.listening` varsa o | kapalı ⚠ |
| `clipboard` | `history` | canlı `clipboard`: geçmiş açık mı, compositor data-control sunuyor mu, izleme çalışıyor mu | protokol yok ⚠ (Hyprland'i güncelle); izleme durdu ⚠ (journalctl); kapalı bilgi |
| | `bridge` | VM pano köprüsü (`[clipboard] x11_bridge`): kurulu mu, devrede mi (`auto`: konuk ajanı bulundu) | kip açık ama çalışmıyor ⚠; `off` ya da konuk ajanı yok bilgi |
| | `sync_tools` | harici X11 ↔ Wayland eşitleyicileri (2026-10-16 gerçek vaka: `~/hypr-bridge.sh` Nuxut köprüsüyle birlikte çalışıyordu). Hyprland ayarı ve bir düzey `source` dosyalarında `exec…` satırları (yorumlar ve `bind` satırları sayılmaz): `hypr-bridge`, `xclip`, `xsel`, `clipnotify`, `clipsync`; çalışan süreçler: `hypr-bridge(.sh)`, `clipnotify`, `clipsync` ve komut satırında eşitleyici izi olan `wl-paste --watch`. Yalnız araç adı ve `dosya:satır`; komut satırı rapora girmez | Nuxut köprüsü `off` değilse ⚠ → satırın başına `#` + oturumu yeniden başlat; `off` iken bilgi (kullanıcının seçimi) |
| | `managers` | başka pano geçmişi yöneticileri (cliphist, clipman, copyq, greenclip, clipse, wl-clip-persist, clipvault; `exec` satırları ve süreçler) | Nuxut geçmişi açıksa ⚠ → satırı `#` ile kapat ya da `[clipboard] enabled = false`; kapalıysa bilgi |
| `system` | `disk` | ev dizininin dosya sisteminde boş alan | < 2 GiB ✗, < 10 GiB ⚠ |
| | `fonts` | temanın ui/display/document/mono/ikon aileleri kurulu mu; eksikse Arch paketi | ⚠ |
| | `packages` | pacman yerel veritabanı (`/var/lib/pacman/local`, yalnız dizin adları): papirus-icon-theme, vimix-cursors, xdg-desktop-portal(-gtk, -hyprland); isteğe bağlı mpv, ddcutil (bilgi). Veritabanı yoksa (Arch dışı) bilgi | ⚠ → "Paket yöneticisiyle kurun: <paketler>" |

## Canlı istek: `msg doctor live` (iç sözleşme, sürüm 1)

Kabuk tek satır JSON döner (alanlar yalnızca eklenir; istemci bilinmeyeni yok sayar):

```json
{"v":1,"pid":1234,"version":"0.1.0","commit":"abc1234","instance":"nuxut-shell",
 "hypr":{…msg hypr status…},"look":{…msg look status…},
 "audio":{"available":true,"backend":"wpctl","sink":"route","source":"props"},
 "lock":{"listening":true},
 "clipboard":{"enabled":true,"data_control":true,"watching":true,"bridge":"auto","bridge_running":true,"bridge_active":false}}
```

`clipboard` (2026-10-16): yalnız durum bayrakları ve köprü kipi; pano içeriği yok.

`audio` ses servisi yoksa `null`. `lock.listening`: `[lock] listen_logind` açık ve logind Lock sinyali
kabuğa ulaşabiliyor (bağlantı, logind, oturum çözümü; c696288). Alan yoksa (eski kabuk)
istemci yalnız ayarı ve ikiliyi denetler. Yanıtı ses thread'i verir (UI beklemez).

## Testler

- `doctor::evaluate` saf: sahte `Facts` ile her denetimin ✓/⚠/✗ sınırları, kişisel veri sızmaz
  (ad/başlık alanı yok), JSON biçimi, çıkış kodu.
- `doctor::gather` sahte ortamla: geçici ev/önek dizinleri, sahte `/proc` kökü, tema paketi
  taraması (dosya:satır, içerik yok), `shell.toml [lock]`, portal dosyaları.
- Oturum çözümü sahte logind ile (`tests/it/session_service.rs` kalıbı).
