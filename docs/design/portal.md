# Portal arka ucu: `xdg-desktop-portal-nuxut`

Durum: v1 (yalnızca `Settings`). Plan: masaüstü boşluk analizi §3; ADR-0004 §7 "kendi portal arka
ucu v2" (B8: `xdg-desktop-portal-gtk` `accent-color` sunmuyor).

## Amaç

libadwaita/GTK4, Qt 6.5+, Firefox, Chromium, Electron ve Flatpak uygulamaları koyu/açık kipi,
vurgu rengini ve azaltılmış hareketi `org.freedesktop.portal.Settings`'ten **canlı** okur. Bu
değerleri Nuxut'un o an çizdiği temadan veren küçük, UI'sız bir arka uç.

## Arayüz

- İkili: `xdg-desktop-portal-nuxut` (kabuk çalışma alanının `portal/` üyesi; `nuxut-theme` +
  `nuxut-config` + `zbus`; renderer/Wayland yok). Kurulum: `$PREFIX/lib/nuxut/`.
- Bus adı `org.freedesktop.impl.portal.desktop.nuxut`, nesne `/org/freedesktop/portal/desktop`,
  arayüz `org.freedesktop.impl.portal.Settings` (`version` = 2).
- `ReadAll(as)`: ad alanı süzgeci — boş liste ya da boş desen hepsi; yalnızca sondaki `*` glob'u
  (`org.freedesktop.*`). `Read(s, s)`: bilinmeyen ad alanı/anahtar →
  `org.freedesktop.portal.Error.NotFound` (xdg-desktop-portal sıradaki arka uca geçer).
  `SettingChanged(s, s, v)`.
- Yalnızca `org.freedesktop.appearance`:

| Anahtar | Tür | Kaynak (`nuxut_theme::Theme`, runtime'ın `ThemeLoader`'ı) |
|---|---|---|
| `color-scheme` | `u` | `Theme::mode`: koyu → 1, açık → 2. `[colors] mode = "auto"` framework'te çözülür (palette.toml kipi → yüzey parlaklığı → koyu); Nuxut her zaman bir kipte çizdiği için 0 (tercih yok) verilmez |
| `accent-color` | `(ddd)` | `Theme::colors.primary`, sRGB 0..1 (8 bit adımına yuvarlanır) |
| `contrast` | `u` | `framework.toml [colors] contrast`: `high` → 1, `standard` → 0 (`Theme::contrast`; 2026-10-14 bulgu, önceden sabit 0). `palette.toml contrast` Material kontrast düzeyidir, erişilebilirlik isteği değil; eşlenmez |
| `reduced-motion` | `u` | `Theme::motion.enabled` yanlışsa 1: Ayarlar → Hareket "Kapalı" (`[motion] preset = "instant"`) ya da `[motion] enabled = false` |

Diğer ad alanları (`org.gnome.desktop.interface` …) bilinmez; `nuxut-portals.conf`'taki
`Settings` listesinde `nuxut`'tan sonraki arka uçtan (tipik olarak gtk; o da look'un yazdığı
dconf'u okur) gelir.

## Kararlar

1. **Tek kaynak framework.** Değerler `ThemeLoader::load()` ile yüklenen `Theme`'den gelir;
   portal renk türetmez, kip çözmez (kopya yok). Bozuk `palette.toml`'da son geçerli palet
   korunur (runtime ile aynı yükleyici).
2. **Birleşik görünüm kapalıysa saydam.** `[desktop] unified = false` (ya da açılışta bozuk
   framework.toml — runtime'ın `broken_config`'i gibi) → hiçbir anahtar bilinmez, değerler gtk
   arka ucundan (kullanıcının kendi dconf'u) gelir. Çalışırken bozulan dosyada son geçerli
   değerler korunur.
3. **Sinyal yalnızca değişen anahtar için.** İki inotify izleyicisi (`nuxut_config::ConfigWatcher`:
   `$XDG_CONFIG_HOME/nuxut/framework.toml`, `$XDG_STATE_HOME/nuxut/palette.toml`; dizin izlenir,
   atomik yeniden adlandırma kaçmaz), `poll(2)` ile beklenir — boşta uyanma yok. Olaydan sonra
   40 ms birleştirme, yeniden yükleme, önceki değerlerle fark; durum güncellendikten SONRA her
   değişen anahtar için bir `SettingChanged`. Saydama geçişte sinyal yok (portal "anahtar kalktı"
   bildiremez); saydamdan etkine geçişte dört anahtar.
4. **Ad devredilmez.** `RequestName(DoNotQueue)`, devir bayrağı yok: ikinci örnek hemen çıkar,
   çalışan örnek yerinden edilmez.
5. **Seçim: `XDG_CURRENT_DESKTOP=Nuxut:Hyprland` + `nuxut-portals.conf`** (karar).
   Seçim dosyası `$XDG_CONFIG_HOME/xdg-desktop-portal/nuxut-portals.conf`; sabit değil, kullanıcının
   Hyprland seçiminden **türetilir** (aşağıda "Seçim dosyası").
6. **`XDG_CURRENT_DESKTOP` Hyprland ortamına** (onay 2026-10-13;
   uwsm oturumu doğrulanmadı — bilinen sınır, ARCHITECTURE "Portal arka ucu"). Hyprland açılışta kendi ortamındaki
   `XDG_CURRENT_DESKTOP` ve `XDG_DATA_DIRS`'i systemd ve D-Bus etkinleştirme ortamına aktarır
   (Hyprland 0.56 `import-environment` + `dbus-update-activation-environment`); environment.d'deki
   değer bununla ezilir. Bu yüzden kabuk değeri üretilen `nuxut.lua`/`nuxut.conf`'a `env` olarak
   yazar (`services::portal_env`, `SyncRequest::session_env`): yalnızca bu oturum, başka masaüstü
   oturumları etkilenmez. Değer `Nuxut:` + önceki liste (varsayılan `Hyprland`), kararlı (ikinci
   üretimde değişmez). Kayıt dosyası (`nuxut.portal`) yoksa hiçbir şey eklenmez.
7. **`.portal` kaydı `$XDG_DATA_HOME`'da.** xdg-desktop-portal 1.22 `.portal` dosyalarını
   `$XDG_DATA_HOME/xdg-desktop-portal/portals` altında da tarar (uçtan uca testte doğrulandı:
   `XDP: load portals from …/data/xdg-desktop-portal/portals`). `XDG_DATA_DIRS`'e dokunulmaz.
8. **Etkinleştirme.** D-Bus servis dosyası (`$XDG_DATA_HOME/dbus-1/services/
   org.freedesktop.impl.portal.desktop.nuxut.service`, `Exec` mutlak yol,
   `SystemdService=xdg-desktop-portal-nuxut.service`) + systemd kullanıcı birimi (`Type=dbus`,
   etkinleştirilmez). systemd yoksa `Exec` doğrudan çalışır.

## Seçim dosyası (`nuxut-portals.conf`)

**xdg-desktop-portal 1.22 davranışı** (kaynak: `src/xdp-portal-config.c`, 1.22.1 etiketi,
`load_portal_configurations` / `load_config_directory`; `man portals.conf`): dizinler sırayla
`$XDG_CONFIG_HOME`, `$XDG_CONFIG_DIRS` (varsayılan `/etc/xdg`), `${sysconfdir}` (`/etc`),
`$XDG_DATA_HOME`, `$XDG_DATA_DIRS` (varsayılan `/usr/local/share:/usr/share`), `${datadir}`
(`/usr/share`); her birinin altında `xdg-desktop-portal/`. Her dizinde `XDG_CURRENT_DESKTOP`
öğeleri (küçük harf) sırayla `<masaüstü>-portals.conf`, sonra `portals.conf`; o dizinden yalnızca
ilk **yüklenebilen** (GKeyFile ayrıştırır, `[preferred]`'da en az bir anahtar) dosya alınır.
Dizinlerin dosyaları zincirlenir: bir arayüz için sırayla her dosyada önce arayüze özel anahtar,
sonra `default` denenir (Settings için `find_all` bulunanları birleştirir); listede `none` arayüzü
kapatır. (`XDG_DESKTOP_PORTAL_DIR` yalnızca xdp'nin kendi testleri içindir; ele alınmaz.)

**Sorun.** `Nuxut:Hyprland` oturumunda `~/.config/xdg-desktop-portal`'da `nuxut-portals.conf`
varken aynı dizindeki `hyprland-portals.conf`/`portals.conf` okunmaz. Sabit dosya kullanıcının
oradaki seçimini (bu makinede `FileChooser = kde`) sessizce gtk'ye çeviriyordu.

**Karar: türetme** (öneri, onay 2026-10-13; `portal/src/portals_conf.rs`, saf + testli; `xdg-desktop-portal-nuxut
--write-portals-conf [--dry-run]`). Nuxut olmasaydı `XDG_CURRENT_DESKTOP=Hyprland` için yüklenecek
ilk dosya yukarıdaki sırayla bulunur (`nuxut-portals.conf` hiç aranmaz) ve kopyalanır: yorumlar,
biçim, bilinmeyen anahtarlar ve diğer gruplar olduğu gibi. Tek değişiklik
`org.freedesktop.impl.portal.Settings` = `nuxut;` + kaynaktaki Settings listesi (yoksa `default`
listesi), tekrarsız, öğeler kırpılmış; `none` atılır (yoksa xdp `nuxut` dahil arayüzü kapatırdı).
Birden çok Settings satırı tek satıra iner (ilkinin yerinde; GKeyFile gibi sonuncunun değeri).
Kopya zincirin en üstüne konduğu ve alt katmanlar Nuxut oturumunda da okunduğu için diğer
arayüzlerin sonucu Hyprland oturumuyla aynıdır. Kaynak yoksa xdp'nin yapılandırmasız Hyprland
davranışı taklit edilir: `UseIn` → hyprland, son çare gtk = `default=hyprland;gtk`,
`Settings=nuxut;hyprland;gtk`. Dosya seçici: `FileChooser=nuxut;<önceki seçim ya da default>` (Nuxut
varsayılan, 2026-10-16; yedek ve `--no-filechooser` geri alma: file-chooser.md). Önceki sabit
`dist/portal/nuxut-portals.conf` kaldırıldı (tek kaynak Rust; şablon ikinci bir doğru üretirdi).

Dosyanın ilk satırı işarettir: sabit önek `# nuxut-managed: portals` + açıklama (`PORTALS_CONF_PREFIX`; sahiplik yalnız öneke bakar, açıklama sürümle değişebilir — şart, 2026-10-13; install.sh `portals_conf_ours` `case … in "$PORTALS_CONF_PREFIX"*` ile aynı; bir
test ikisini karşılaştırır), ardından kaynak dosyanın yolu ve yeniden üretim komutu yorum olarak.
Yazım yalnızca hedef yoksa ya da işaret duruyorsa, aynı dizinde geçici dosya + `rename` ile
(atomik); içerik aynıysa dokunulmaz. İşaret silinmişse dosya kullanıcınındır: yazılmaz, kaldırma
da silmez. Kullanıcının dosyaları yalnızca okunur.

**Yeniden üretim.** Kullanıcı `hyprland-portals.conf`'unu (ya da kaynak olan dosyayı) değiştirirse
`~/.local/lib/nuxut/xdg-desktop-portal-nuxut --write-portals-conf` (ya da install.sh) çalıştırır;
etkisi bir sonraki girişte (xdp yalnızca açılışta okur). `--dry-run` içeriği stdout'a basar,
hiçbir şey yazmaz.

**Açık nokta: otomatik tazeleme v1 dışı.** Kabuk açılışında yeniden üretmek mümkün ama: (1) xdp
dosyayı yalnızca kendi açılışında okur ve oturumda kabuktan önce başlayabilir — tazeleme o oturuma
yetişmeyebilir, kullanıcı için davranış "bazen bir giriş gecikmeli" olur; (2) kabuk, kurulumun
sahibi olduğu bir dosyayı her açılışta yazmaya başlar (kurulum/kaldırma sınırı bulanıklaşır);
(3) yanlış bir türetme sessizce her girişte yinelenir. İhtiyaç doğarsa: kabuk açılışında yalnızca
kaynak dosyanın mtime'ı kopyadan yeniyse yeniden üret + Ayarlar'da bilgi; ya da
xdg-desktop-portal.service için bir drop-in `ExecStartPre` (xdp okumadan hemen önce üretir).

## Oturum boyu yeniden başlatma

xdg-desktop-portal `XDG_CURRENT_DESKTOP`'u, portals.conf'u ve `.portal` listesini **yalnızca
açılışta** okur. Kurulum ya da güncelleme çalışan portalı yeniden başlatmaz (kullanıcının
oturumundaki uygulamalar bağlı): etkisi bir sonraki girişte. Elle hemen denemek isteyen:
Hyprland ortamı aktarıldıktan sonra `systemctl --user restart xdg-desktop-portal`.

## Testler

- Birim (`portal/src`): kip → `color-scheme` (auto + duvar kağıdı paleti dahil), primary →
  `accent-color` (`(ddd)`, 0..1), hareket → `reduced-motion`, saydam kip, fark, glob kuralları.
- Özel `dbus-daemon` (`portal/tests/settings_dbus.rs`; dbus-daemon yoksa PANİK, atlama yalnızca
  `NUXUT_PORTAL_SKIP_DBUS_TESTS=1`): ReadAll glob, Read, `NotFound`, `version`, saydam kip, dosya
  değişince değişen anahtar başına tek sinyal (aynı içerik → sinyal yok; bozuk dosya → son değer),
  `palette.toml` izleme, ikinci örneğin adı alamaması.
- Seçim dosyası (`portals_conf`, geçici dizinler): arama sırası (config home > config dirs >
  sysconfdir > data home > data dirs > datadir; dizinde masaüstü dosyası `portals.conf`'tan önce;
  `nuxut-portals.conf`, anahtarsız ve ayrıştırılamayan dosya atlanır), anahtar/yorum korunumu
  (`--no-filechooser` ile `FileChooser = kde` kalır; varsayılanda `nuxut;kde` + yedek grubu), Settings birleştirme (Settings var/yok/zaten `nuxut`/`none`/yinelenen
  anahtar), kaynak yok, işareti silinmiş dosyaya dokunmama, atomik yazım (yeni inode), install.sh
  işaretiyle eşlik.
- Kabuk: `services::portal_env` (saf), `hypr_config::sync` (oturum ortamı modüle imleç ortamından
  önce girer).
- Uçtan uca (elle; geliştirici betiği): `dbus-run-session` içinde gerçek
  `/usr/lib/xdg-desktop-portal`, geçici `HOME`/`XDG_*`/`XDG_RUNTIME_DIR`, `DCONF_PROFILE`,
  `GDK_DEBUG=no-portals`, test compositor'ı; `XDG_CURRENT_DESKTOP=Nuxut:Hyprland`, kullanıcınınki
  gibi bir `hyprland-portals.conf` (`default=gtk`) da varken `nuxut-portals.conf` seçilir;
  `org.freedesktop.portal.Settings.ReadOne` değerlerimizi döner, framework.toml değişince
  `SettingChanged` xdg-desktop-portal üzerinden uygulamaya ulaşır.

## Sonra

FileChooser (Nuxut görünümlü seçici), ScreenCast paylaşım seçicisi.
