# Birleşik görünüm: GTK canlı renkler, devralma, kitty, Qt stili, klasör ikonları

Durum: uygulandı (2026-10-16). İlgili: ADR-0004 §5.2 (bu belge oradaki
"GTK3 renkleri yalnızca açılışta" kararını değiştirir), `src/services/look/gtk_theme.rs`,
`plan.rs`, `docs/design/look-external.md` §4 (devralma).

## Sorun

Kullanıcı: "GTK hiç değişmiyor". Gerçek makinede iki neden:

1. Renkler kullanıcı `~/.config/gtk-{3,4}.0/gtk.css`'ine `@import url("nuxut.css")` satırıyla
   veriliyordu. GTK bu dosyayı **yalnızca açılışta** okur: açık uygulamalar hiç değişmez.
2. `~/.config/gtk-4.0/gtk.css` başka bir temaya (Dracula) sembolik bağ; Nuxut dokunmuyordu
   (`not_owned`). Devralma artık varsayılan (kullanıcı ilkesi 2026-10-16, look-external.md §4).

## Ölçümler (izole sway + yalıtılmış dconf, GTK 3.24.52 / 4.24.1 / libadwaita 1.10)

| Deney | Sonuç |
|---|---|
| GTK3 (`gspell-app1`): `gtk-theme` A → B (ayrı tema dizinleri) | Canlı değişti |
| Aynı ad, tema CSS'i değişti, aynı değer yeniden yazıldı | Değişmedi (değişim sinyali yok) |
| Kullanıcı `gtk.css` aynı adla `@define-color` tanımlıyor (açılışta var) | Kullanıcınınki kazanır, tema rengi gölgelenir |
| libadwaita (`gnome-calculator`): `gtk-theme` | Yok sayılır (beklenen) |

## Karar

**GTK3 (ve libadwaita'sız GTK4 uygulamaları) — Nuxut teması:**

- `$XDG_DATA_HOME/themes/Nuxut/` (look.mode'un yönetilen dosyaları, `nuxut-apply`):
  - `gtk-3.0/base.css`: taban temanın (adw-gtk3 / adw-gtk3-dark, kipe göre) CSS'i, **bütün renk
    adları `nuxut_` önekiyle** yeniden adlandırılmış, göreli `url()`'ler taban dizine mutlak.
    Böylece kullanıcının (ör. matugen) `gtk.css`'indeki `@define-color`'lar tema kurallarına hiç
    ulaşmaz — kullanıcının dosyasına dokunmadan gölgeleme kalkar. Kullanıcının renk dışı kendi
    kuralları yine uygulanır.
  - `gtk-3.0/gtk.css`: `@import url("base.css")` + Nuxut renkleri (`@define-color nuxut_…`).
  - `gtk-4.0/gtk.css`: taban GTK 4 CSS'ine `@import`.
  - `index.theme`.
- İkiz `themes/Nuxut-reload/` aynı içeriği asıl temadan `@import` eder. Renk değişince canlı
  eylem `gtk_reload`: dconf `gtk-theme` önce `Nuxut-reload`, hemen sonra `Nuxut` — GTK iki kez
  yeniden yükler, iki görüntü aynı (yanıp sönme yok). Son değer `nuxut-apply`'ın yazdığıyla aynı.
- `gtk-theme` (dconf) ve `settings.ini gtk-theme-name` = `Nuxut`.
- Kullanıcı `gtk-3.0/gtk.css`'indeki önceki `@import url("nuxut.css")` satırı ve
  `gtk-3.0/nuxut.css` kaldırılır (kalsalar açılışta yüklenip temayı gölgelerdi).
- Taban tema (adw-gtk3) kurulu değilse ya da CSS'i okunamıyor/`@import` içeriyorsa önceki yol:
  `gtk-theme` = Adwaita, kullanıcı `gtk.css` satırı (yeniden başlatınca).

**GTK4 / libadwaita:** libadwaita temayı yok sayar; renkler yalnızca kullanıcı
`gtk-4.0/gtk.css` → `nuxut.css` (açılışta). Canlı olanlar: koyu/açık ve vurgu rengi
(Nuxut portalı `org.freedesktop.appearance` `accent-color`, libadwaita ≥ 1.6 keyfi RGB vurguyu
canlı uygular). Yüzey renkleri için uygulama yeniden açılmalı (`restart_needed: gtk4_colors`).
Gerçek Nautilus 51 / libadwaita 1.10 ile ölçüldü (2026-10-17, izole oturum): açık pencere de,
son pencere kapanınca arka planda kalan `--gapplication-service` süreci de eski CSS'i tutar;
yeni renkler `nautilus -q` ve yeniden açınca gelir. libadwaita 1.10 hem `--window-bg-color` gibi
CSS değişkenlerini hem `@define-color`'ı uygular; `gtk.css` sembolik bağsa izlenir.
Nautilus'ta görünen yüzeyler içerik ve kenar çubuğudur (`view_bg`/`sidebar_bg` →
`surface_container_low`, ADR-0004 §5.1). Elle yazılmış tema yüzeylerinde (`framework.toml
[colors]`) bu rol tohumun nötr tonunda kalıyordu (Dracula'da `#191c20`, varsayılan Adwaita
koyusuyla neredeyse aynı); artık token'ı olmayan yüzey basamakları temanın üç yüzeyinden kurulur
(`look/palette.rs` `fit_surface_ramp`).

**Devralma (varsayılan; kullanıcı ilkesi 2026-10-16 "Nuxut yapabiliyorsa varsayılan Nuxut"):**
`writer::ApplyBackend` `nuxut-apply`'dan `ExternallyManaged { reason }` alınca, neden
devralınabiliyorsa (`ExternalReason::can_take_over`: son bileşeni symlink olan yol — hedefi
root'un tema dosyası, salt okunur ya da kırık olsa da —, salt okunur ya da sabit bağlı düz dosya)
isteğe `Change::take_over(yol)` ekleyip yeniden dener. Özgün biçim (`symlink:<metin>`,
`mode:<sekizli>`, `hardlink`) aynı snapshot'a girer; "önceki hale dön" ve birleşik görünümü
kapatmak bağı aynı metinle, izni ve sabit bağı geri koyar. Bağın hedefine hiç yazılmaz.
Devralınamaz: nix/guix deposu (bildirimsel yapılandırma), başka kullanıcının düz dosyası,
yazılamayan dizin — bunlar yine `last_apply.external` (Ayarlar uyarır, `not_owned` için
`chown` önerir).

**Tema bağı olan kullanıcı CSS'i:** Kullanıcının makinesinde `~/.config/gtk-4.0/{gtk.css,
gtk-dark.css,assets}` → `/usr/share/themes/Dracula/gtk-4.0/…` bağları vardı (Dracula'nın
libadwaita kurulumu). Bağ devralınıp içerik kopyalansa Dracula'nın tam CSS'i Nuxut renklerini
ezmeye devam ederdi (kendi sabit renkleri). Bu yüzden `gtk-4.0/gtk.css`, `gtk-4.0/gtk-dark.css`
ve `gtk-3.0/gtk.css` başka bir temaya devralınabilir bağsa (`SystemProbe::foreign_link`:
symlink + `external_reason` devralınabilir; kullanıcının kendi yazılabilir dosyasına bağ değil)
plan o dosyayı Nuxut'un yönetilen dosyası yapar: yalnızca başlık + `@import url("nuxut.css")`
(GTK3 Nuxut temasıyla yalnızca başlık). `assets` dizin bağına dokunulmaz (artık kimse
başvurmuyor; kapatınca CSS bağları geri gelince yine işe yarar). Bir sonraki turda dosya düz
dosya olduğundan plan olağan satır parçasına döner (satır zaten var → NoOp).

**Görünür durum:** `msg look status` sürüm 6 `last_apply.taken_over` (`{ setting, path }`):
uygulanmış (geri yüklenmemiş) `look.*` snapshot'larındaki `file_form` parçaları, yol bugün yine
bağ değilse (kullanıcı bağı elle geri kurduysa artık Nuxut'un değil). Shell yeniden başlasa da
kayıt depodan okunur. Günlükte devralma anında tek INFO satırı (`görünüm: dosya devralınıyor`).

## kitty (kullanıcı isteği 2026-10-16)

`kitty` `PATH`'teyse: `~/.config/kitty/nuxut.conf` (`look.mode`; framework'ün kitty şablonu
seçili paletle — 16 ANSI rengi birincil renge uyumlandırılmış) ve `nuxut-motion.conf`
(`look.motion`; `cursor_trail 3` + `cursor_trail_decay 0.1 0.4`, azaltılmış harekette
`cursor_trail 0`). Kullanıcının `kitty.conf`'una sona iki `include` satırı (dosya yoksa yalnızca
onlarla yaratılır; kitty'de sonraki tanım kazanır, `kitten themes` bloğu da ezilir). Canlı:
`kitty_reload` bu kullanıcının en az 2 sn'dir çalışan `kitty` süreçlerine `SIGUSR1` (yeni açılan
süreç işleyicisini kurmadan sinyal alırsa kapanırdı). Test örneği ve kapalı ortamda sinyal yok
(`live_gate`).

## Qt: Nuxut stili (ADR-0009)

`lib/nuxut/qt6/styles/libnuxut-style.so` kuruluysa (install.sh derler; arama: çalışan ikilinin
`../lib/nuxut`, `~/.local/lib/nuxut`, `/usr/lib/nuxut`): kdeglobals `widgetStyle=Nuxut` (qt6ct
yolunda `style=Nuxut`), `$XDG_CONFIG_HOME/nuxut/qt-style.conf` (`look.mode`: köşe yarıçapları,
odak halkası 2 px birincil renk, 4 px ızgara) ve `QT_PLUGIN_PATH=<lib>/qt6:<lib>/qt5`. Değilse
Breeze (yoksa Fusion), değişken yazılmaz.

## Klasör ikonları (kullanıcı isteği 2026-10-16)

Papirus ailesi seçiliyken (`Papirus`, `Papirus-Dark`, `Papirus-Light`) klasörler paletle renklenir
(`look::folders`). Papirus her klasörü renk çeşitleriyle getirir ve genel adları (`folder.svg`,
`folder-documents.svg`, `user-home.svg`) bağla maviye verir; `papirus-folders` bu bağları root
olarak değiştirir ve yalnızca 18 sabit renk bilir. Nuxut sistem dosyalarına dokunmaz:

- `$XDG_DATA_HOME/icons/<tema>-Nuxut/` (`look.icons`, `nuxut-apply`; gizli `index.theme`,
  `Inherits=<tema>,hicolor`): mavi çeşide bağlı her genel adın kopyası, Papirus mavisinin üç tonu
  (`#5294e2` ön, `#4877b1` arka, `#1d344f` simge) vurguya taşınmış — HCT'de ton aynen, açı
  vurgunun, renklilik mavininkiyle orantılı ama vurgudan fazla değil (nötr/grafit temada gri
  klasör). Açı 5°, oran 0,1 adımla yuvarlanır: küçük duvar kâğıdı farkları ikonları yeniden
  yazdırmaz (NoOp). Vurgu açık kipin `primary`'si (iki kipte aynı klasör rengi, Papirus gibi).
- İki tasarım boyu ölçeklenebilir dizinde: 22 px tasarımı 20–31, 48 px tasarımı 32–512 (≈ 2 × 85
  dosya; look.icons tek işleme sığar, boy başına en çok 110). 16 px Papirus'un kendisi (renk
  çeşidi yok).
- Yazılan ikon teması adı (GTK `settings.ini`, dconf, kdeglobals, qt6ct) `<tema>-Nuxut`. Canlı:
  KDE `iconChanged` + `icon_reload` (dconf `icon-theme` önce içeriksiz ikiz
  `<tema>-Nuxut-reload`'a — asıl Nuxut temasını miras alır, görüntü aynı — sonra asıl ada; GTK
  aynı adla yeniden yazılan dosyaları kendiliğinden okumaz). Ölçüldü (2026-10-17, izole sway +
  yalıtılmış dconf/HOME, GTK3 evince dosya seçicisi): gül → adaçayı yeniden yazımından 8 sn sonra
  klasörler hâlâ gül; ikiz → asıl `icon-theme` geçişinden hemen sonra adaçayı. İkiz şart ve yeterli.
- Yükleme bloklayan IO (≈ 170 küçük SVG): görünüm thread'inde ve `msg look status` yolunda
  dizinlerin değişim zamanına göre önbellekli (taban GTK CSS'i de aynı biçimde).
- Başka ikon teması seçiliyse hiçbiri yazılmaz; birleşik görünümü kapatmak temayı siler.

## Renk temaları (framework `[colors] theme`)

Statik kaynakta bir Nuxut renk teması seçiliyken dış uygulama paleti tohumdan türetilmez:
`Scheme::from_color_theme` iki kipi de temanın elle ayarlanmış tonal paletlerinden kurar
(`palette::Seed::with_color_theme`). Önceki tohumlama açık kipte birincilden türetilen küçük bir
kayma taşıyordu; artık Nuxut'un çizdiği 11 token ile dışa aktarılan roller iki kipte de aynı.
Duvar kâğıdı kaynağında tohum yine `palette.toml`'dan.

## GTK2

Yönetilmez. GTK2 uygulaması neredeyse kalmadı; Nuxut renklerini taşıyacak bir GTK2
motoru (adw-gtk3'ün GTK2 sürümü yok) yazmak maliyetine değmez. Kullanıcının GTK2 teması
(Dracula) korunur.
