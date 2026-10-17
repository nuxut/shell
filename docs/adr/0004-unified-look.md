# ADR-0004 — Birleşik görünüm: GTK, Qt, ikon, imleç ve yazı tipi tek kaynaktan

Tarih: 2026-10-10 · Durum: **kabul edildi** (kararlar §12, 2026-10-10) · Pano: t083
Kardeş kararlar: ADR-0001 (dış yapılandırma uygulama + snapshot katmanı,
`nuxut-apply`), ADR-0002, ADR-0003 (Hyprland Lua ayarı ve başlatma),
ADR-0005; tasarım notu `docs/design/hyprland-sync.md`.

## 1. Bağlam

Kullanıcı: "Ekstra bir iş değil; temel ayarların üstüne gerçekten birleşik görünüm istiyorum;
end4 bunu güzel yapmış." Nuxut Ayarlar'da imleç, ikon paketi, renk, koyu/açık ve yazı tipi
seçilince bu **her yerde** değişmeli: GTK3, GTK4/libadwaita, Qt5/Qt6, KDE uygulamaları,
Hyprland imleci, XWayland. Kullanıcı her araç için ayrı ayar yapmayacak.

Bu VM'nin bugünkü hâli sorunun canlı örneği (yalnızca okundu, 2026-10-10):

| Kaynak | İmleç | İkon | Qt |
|---|---|---|---|
| `hyprland.conf` `env`/`exec-once` | `HYPRCURSOR_THEME=Vimix-cursors`, `XCURSOR_THEME=Bibata`, `setcursor Vimix-cursors 24` | — | — |
| `hypr/configs/default/env.conf` | `XCURSOR_THEME=Vimix-cursors` | — | `QT_QPA_PLATFORMTHEME=qt5ct`, `_QT6=qt6ct`, `QT_STYLE_OVERRIDE=kvantum` |
| `environment.d/10-nuxut-cosmetics.conf` (eski proje) | — | — | `QT_QPA_PLATFORMTHEME=kde`, `QT_STYLE_OVERRIDE=Darkly` |
| dconf `org.gnome.desktop.interface` | `Vimix-cursors` (sonra `Bibata` okundu; yazanı belirlenmedi) | `elementary` | — |
| `gtk-3.0/4.0/settings.ini` | `Bibata-Modern-Classic` | `Papirus-Dark` (**kurulu değil**) | — |
| `~/.icons/default/index.theme` | `Inherits=Bibata-Modern-Classic` | — | — |
| `qt6ct.conf` / `kdeglobals` | — | `Papirus-Dark` | `style=kvantum` / `widgetStyle=Darkly` |

Yedi kaynak, üç farklı imleç, üç farklı ikon teması (biri kurulu değil), iki çelişen Qt
yolu. Bu ADR bunu **tek kaynak → üretilen hedefler** mimarisiyle çözer.

## 2. Referanslar nasıl yapıyor

**end4 (dots-hyprland, illogical-impulse).** Duvar kağıdı → `matugen` (Rust ikilisi) →
şablonlar: `~/.config/gtk-3.0/gtk.css` ve `gtk-4.0/gtk.css` doğrudan **ezilir**
(`@define-color` + GTK4'te `@media (prefers-color-scheme)` blokları + düğme/yol çubuğu yuvarlatma
CSS'i). GTK teması `adw-gtk3`/`adw-gtk3-dark` (`gsettings gtk-theme`, `color-scheme`).
Qt: `QT_QPA_PLATFORMTHEME=kde` (plasma-integration) + `kdeglobals` `widgetStyle=Darkly`;
renkler Python aracı `kde-material-you-colors` ile `kdeglobals`'a. İmleç: `exec-once =
hyprctl setcursor Bibata-Modern-Classic 24`; ikon `breeze-dark` (`kdeglobals [Icons]`).
Yazı tipi: kurulumda bir kez `gsettings font-name` + `kdeglobals` fontları. Geri alma yok;
kullanıcı dosyaları ezilir.

**noctalia.** Şablon motoru + uygulama/geri alma kancaları (`builtin.toml`'da her şablonun
`post_hook` ve `undo_hook`'u). GTK: renkler ayrı `noctalia.css`'e, kullanıcının `gtk.css`'ine
yalnızca `@import url("noctalia.css");` satırı eklenir (geri almada bu satır silinir) —
**en saygılı yöntem**. `gsettings gtk-theme adw-gtk3[-dark]` (tema kuruluysa) + `color-scheme`.
Qt: `qt5ct/colors/noctalia.conf`, `qt6ct/colors/noctalia.conf` (yalnızca renk şeması dosyası;
seçimi kullanıcı yapar). KDE: `color-schemes/noctalia.colors` üretilir, `kdeglobals`'a
**birleştirilir** (yalnızca şemadaki anahtarlar; diğer satırlar korunur, atomik, izin
korunur) ve D-Bus `org.kde.KGlobalSettings.notifyChange(0,0)` sinyali yayınlanır (canlı).
İmleç/ikon yönetmez.

**caelestia-shell.** Shell deposu yalnızca kendi QML temasını taşır; GTK/Qt dışa aktarımı
ayrı `caelestia-cli` aracında (referanslarda yok). Bu ADR'ye katkısı yok.

**Eski Nuxut (`nuxut-cosmetics`).** Bu VM'de izleri var: `~/.local/share/themes/Nuxut-…`
GTK teması, Kvantum teması, `qt6ct/colors/…`, `environment.d`, `~/.icons/default`. Çelişkiler
yukarıdaki tabloda; tek yazıcı ve geri alma olmadığı için kaynaklar birbirinden koptu.

**Framework zaten yarısını yapıyor.** `nuxut-palette` (material-colors crate'i = matugen'in
saf Rust eşdeğeri) duvar kağıdından/renkten Material şeması üretir ve
`$XDG_STATE_HOME/nuxut/exports/{gtk3.css,gtk4.css,qt6ct-colors.conf,kdeglobals-colors,…}`
yazar; ama kullanıcı config'lerine **dokunmaz** ("`@import` satırını kullanıcı ekler").
Statik temada (duvar kağıdı yokken) yalnızca 11 token var, açık kip yok.

## 3. Doğrulanmış bulgular (bu VM, izole headless sway + izole D-Bus + geçici XDG)

Sürümler: gtk3 3.24.52, gtk4 4.22.5, libadwaita 1.9.4, qt6-base 6.11.2, qt5ct 1.9, qt6ct 0.11,
plasma-integration 6.7.5, adw-gtk-theme (extra) 6.5, Hyprland 0.56.2, hyprcursor 0.1.13.

| # | Deney | Sonuç |
|---|---|---|
| B1 | libadwaita (gnome-calculator), `gtk-4.0/gtk.css` içinde hem `@define-color window_bg_color` hem `:root { --window-bg-color }` | **Yalnızca CSS değişkeni** etkili. libadwaita ≥ 1.6 `@define-color`'ı bu roller için yok sayıyor. Üretici CSS değişkenlerini yazmalı (end4/framework şablonları şu an `@define-color` yazıyor → libadwaita'da etkisiz). |
| B2 | GTK3 (evince, `adw-gtk3-dark`), `gtk-3.0/gtk.css` `@define-color` | Etkili. |
| B3 | Çalışırken `gtk.css` değiştirildi (GTK3 ve GTK4) | **Canlı değil**: ikisi de dosyayı yalnızca açılışta okur. |
| B4 | `gtk-4.0/gtk.css` içinde `@media (prefers-color-scheme: dark/light) { :root {…} }`, sonra `color-scheme` `prefer-dark` → `prefer-light` (portal üzerinden) | **Canlı**: libadwaita uygulaması anında diğer bloğa geçti. Koyu/açık geçişi GTK4'te yeniden başlatma istemez; yeter ki iki kipin renkleri dosyada önceden olsun. |
| B5 | Qt6 + `QT_QPA_PLATFORMTHEME=qt6ct` (qdbusviewer6, Fusion), çalışırken `qt6ct.conf`'ta `color_scheme_path` ve şema dosyasının içeriği değişti | **Canlı değil.** |
| B6 | KDE uygulaması (kwrite) + `QT_QPA_PLATFORMTHEME=kde` (plasma-integration), `kdeglobals` `[Colors:*]` değişti + `notifyChange(0,0)` sinyali | **Canlı**: palet anında değişti. |
| B7 | `gsettings gtk-theme` değişimi ile GTK3 renklerini canlı çevirme (tema dizini A/B) | **Sonuçsuz** (izole testte GTK3 Wayland ayarı okumadı; §16 adım 0). Şimdilik GTK3 renkleri = yeniden başlatma. |
| B8 | libadwaita vurgu rengi, `accent-color` gsettings değişimi | Etkisiz: `xdg-desktop-portal-gtk` bu VM'de `accent-color` sunmuyor (portals.conf: `hyprland;gtk`). |

Test tuzağı (kayda değer): `dbus-run-session` geçici `XDG_CONFIG_HOME` ayarlanmadan **önce**
başlatılırsa D-Bus ile etkinleşen `dconf-service` ve portal **gerçek** `~/.config/dconf/user`'a
yazar; istemci ise geçici dosyayı okur. İlk denemelerde bu yüzden kullanıcının dconf'unda
`gtk-theme`, `color-scheme`, `accent-color` yaklaşık 3 dk değişti, sonra özgün değerlerine
döndürüldü. Kural (§15): XDG dizinleri D-Bus oturumundan önce ayarlanır, betik
`XDG_CONFIG_HOME`'un geçici olduğunu doğrular.

## 4. Karar (özet)

1. **Tek kaynak:** framework teması (`framework.toml` + `palette.toml`) ve yeni masaüstü
   seçimleri (koyu/açık, ikon teması, imleç teması/boyu, birleşik görünüm açık/kapalı).
   Kodda sabit renk yok; tüm renkler temadan gelir.
2. **Üreticiler saf ve shell'de** (`services/look`; karar, §17-6): `Theme` + seçimler
   → ayar başına `(Fragment, değer)`. Dış yazımları yalnızca shell yaptığı için tek tüketici shell;
   Ayarlar önizlemeyi `msg look plan` ile okur. Rol türetmesi framework'te (`nuxut-palette`). matugen **kullanılmaz** (§10).
3. **Yazımlar yalnızca `nuxut-apply` katmanından** geçer (atomik, parça düzeyinde snapshot,
   geri yükleme). Bu ADR yalnızca hangi parçaların yönetildiğini söyler (§6).
4. **Shell çalışma anı sahibidir:** tema/seçim değişimini izler (zaten izliyor), üreticiyi
   çalıştırıp uygular, ardından **dosya dışı canlı eylemleri** yapar (§7): Hyprland imleci,
   systemd/D-Bus etkinleştirme ortamı, KDE sinyalleri. Ayarlar yalnızca `framework.toml`
   yazar ve önizleme/plan/geri yükleme gösterir (karar §12-1).
5. **Kullanıcının dosyalarına saygı:** mümkün olan her yerde ayrı yönetilen dosya + tek
   işaretli satır (noctalia yöntemi); kullanıcı dosyasında yalnızca yönetilen anahtar değişir.
6. **Qt yolu:** `QT_QPA_PLATFORMTHEME=kde` (plasma-integration) + `kdeglobals` — tek canlı
   Qt yolu (B6). qt6ct/qt5ct yalnızca plasma-integration kurulamazsa yedek (§5.3).
7. **Varsayılanlar:** ikon Papirus (koyu/açık kipe göre `Papirus-Dark`/`Papirus-Light`), imleç
   Vimix-cursors 24 (yoksa Adwaita), GTK3 teması `adw-gtk3[-dark]`, Qt stili Breeze.
8. **İlk açılışta kısa Hoş geldin** (3 adım); "Birleşik görünüm" önerilen seçenektir ve
   mevcut ayarları ezer, ama her şey geri alınabilir (§9).

## 5. Hedefler ve eşleme sözleşmesi

### 5.1 Renk rolleri

Kaynak adlar Material 3 rolleri (framework `nuxut-palette` adları). Statik temada bugün
yalnızca 11 token var; tam rol kümesi framework'ten istenir (§13-1). Tabloda `kip` = etkin
kip; GTK4 dosyası **iki kipi de** taşır (B4).

| Anlam | Material rolü | GTK4/libadwaita CSS değişkeni | GTK3 (adw-gtk3) `@define-color` | QPalette rolü | kdeglobals |
|---|---|---|---|---|---|
| Pencere zemini | `surface` | `--window-bg-color` | `window_bg_color`, `theme_bg_color` | Window | `[Colors:Window] BackgroundNormal` |
| Pencere metni | `on_surface` | `--window-fg-color` | `window_fg_color`, `theme_fg_color` | WindowText | `[Colors:Window] ForegroundNormal` |
| İçerik (liste, metin alanı) | `surface_container_low` | `--view-bg-color` | `view_bg_color`, `theme_base_color` | Base | `[Colors:View] BackgroundNormal` |
| İçerik metni | `on_surface` | `--view-fg-color` | `view_fg_color`, `theme_text_color` | Text | `[Colors:View] ForegroundNormal` |
| Alternatif satır | `surface_container` | — | — | AlternateBase | `[Colors:View] BackgroundAlternate` |
| Başlık çubuğu | `surface_container` | `--headerbar-bg-color`, `--headerbar-backdrop-color`(=`surface`) | `headerbar_bg_color`, `headerbar_backdrop_color` | — | `[Colors:Header] BackgroundNormal` |
| Başlık metni | `on_surface` | `--headerbar-fg-color` | `headerbar_fg_color` | — | `[Colors:Header] ForegroundNormal` |
| Başlık gölgesi | `shadow` (α 0.36 koyu / 0.07 açık) | `--headerbar-shade-color`, `--shade-color` | `shade_color` | Shadow | — |
| Kenar çizgisi | `outline_variant` | `--headerbar-border-color`, `--sidebar-border-color` | `borders`, `unfocused_borders` | Mid | `DecorationHover` (Window/View) |
| Kenar çubuğu | `surface_container_low` | `--sidebar-bg-color`, `--sidebar-backdrop-color` | `sidebar_bg_color` | — | `[Colors:Complementary] BackgroundNormal` |
| İkincil kenar çubuğu | `surface_container` | `--secondary-sidebar-bg-color` | `secondary_sidebar_bg_color` | — | — |
| Kart | `surface_container` | `--card-bg-color` / `--card-fg-color` (`on_surface`) | `card_bg_color` | — | — |
| Açılır pencere, menü | `surface_container_high` | `--popover-bg-color`, `--popover-fg-color` | `popover_bg_color` | — | — |
| İletişim kutusu | `surface_container_high` | `--dialog-bg-color`, `--dialog-fg-color` | `dialog_bg_color` | — | — |
| Düğme | `surface_container_high` | (libadwaita türetir) | — | Button / ButtonText (`on_surface`) | `[Colors:Button] BackgroundNormal/ForegroundNormal` |
| Vurgu | `primary` | `--accent-bg-color`, `--accent-color` | `accent_bg_color`, `accent_color`, `theme_selected_bg_color` | Highlight, Accent (Qt ≥ 6.6) | `[Colors:Selection] BackgroundNormal`, tüm grupların `DecorationFocus` |
| Vurgu üstü metin | `on_primary` | `--accent-fg-color` | `accent_fg_color`, `theme_selected_fg_color` | HighlightedText | `[Colors:Selection] ForegroundNormal` |
| Bağlantı | `primary` / ziyaret edilmiş `tertiary` | (vurgudan) | `link_color` | Link / LinkVisited | `ForegroundLink` / `ForegroundVisited` |
| Pasif metin | `on_surface_variant` | (libadwaita `--dim-opacity`) | `insensitive_fg_color` | PlaceholderText, `disabled` grubu | `ForegroundInactive` |
| Yıkıcı/hata | `error`, `on_error`, `error_container`, `on_error_container` | `--destructive-bg-color`, `--destructive-fg-color`, `--destructive-color`, `--error-*` | `destructive_*`, `error_*` | — | `ForegroundNegative` |
| Uyarı | `warning`* | `--warning-bg-color` … | `warning_*` | — | `ForegroundNeutral` |
| Başarı | `success`* | `--success-bg-color` … | `success_*` | — | `ForegroundPositive` |
| İpucu balonu | `inverse_surface` / `inverse_on_surface` | — | — | ToolTipBase / ToolTipText | `[Colors:Tooltip]` |
| Sekme/başlık üstü metin (Selection Inactive) | `primary_container` / `on_primary_container` | — | `theme_unfocused_selected_*` | `inactive` grubunda Highlight | `[Colors:Selection] BackgroundAlternate` |

\* Nuxut temasında `success`/`warning` yok. Sabit renk yazılmaz: framework bu rolleri
ekleyene kadar (§13-2) bu değişkenler **hiç yazılmaz** ve uygulamanın kendi varsayılanı kalır.

QPalette üç grubu: `active` = tablo; `inactive` = aynı, Highlight → `primary_container`;
`disabled` = metin rolleri `outline`, Highlight `surface_container_highest`. Qt ≥ 6.6 `Accent`
rolü = `primary`.

kdeglobals ek anahtarları: `[General] ColorScheme=Nuxut` (+ `[KDE] contrast`), her grupta
12 anahtarın tamamı (eksik anahtarda KColorScheme Breeze'e düşer), `[ColorEffects:Inactive]
Enable=false`, `[ColorEffects:Disabled]` Material'e uygun sabit katsayılar (renk değil
katsayı: `ContrastAmount`, `IntensityAmount`).

### 5.2 GTK

| Hedef | Tür (`nuxut-apply` `Fragment`) | İçerik |
|---|---|---|
| `~/.config/gtk-4.0/nuxut.css` | `ManagedFile` | İki kip: `@media (prefers-color-scheme: dark) { :root { --…: …; } }` ve `light` bloğu (B1, B4). İsteğe bağlı v2: köşe yarıçapı (`shape`) → düğme/kart `border-radius` (end4'ün yaptığı gibi). |
| `~/.config/gtk-4.0/gtk.css` | `Line` (`@import url("nuxut.css");`, dosyanın **sonuna**) | Kullanıcının kendi CSS'i korunur. Satır yoksa eklenir; yorumlanmışsa eklenmez (kullanıcı kararı). |
| `~/.config/gtk-3.0/nuxut.css` | `ManagedFile` | **Etkin kip** için `@define-color` (GTK3'te `@media` yok); kip değişince yeniden yazılır, GTK3 uygulamaları yeniden başlatılınca alır. |
| `~/.config/gtk-3.0/gtk.css` | `Line` (`@import url("nuxut.css");`) | Aynı kural. |
| `~/.config/gtk-3.0/settings.ini`, `gtk-4.0/settings.ini` | `IniKey` `[Settings]`: `gtk-theme-name`, `gtk-icon-theme-name`, `gtk-cursor-theme-name`, `gtk-cursor-theme-size`, `gtk-font-name`, `gtk-application-prefer-dark-theme` | XWayland/xsettings'siz GTK ve ayarı yalnızca buradan okuyanlar (framework `IconTheme::system()` dahil) için yedek. Diğer anahtarlar korunur. |
| dconf `/org/gnome/desktop/interface/` | `Dconf`: `color-scheme` (`prefer-dark`/`prefer-light`), `gtk-theme` (`adw-gtk3-dark`/`adw-gtk3`), `icon-theme`, `cursor-theme`, `cursor-size`, `font-name`, `document-font-name`, `monospace-font-name` | GTK3/GTK4 ve portal (libadwaita `color-scheme`) bunları canlı okur. `accent-color` yazılmaz (yalnızca 9 adlandırılmış renk; B8). |

Var olan, başlıksız yabancı içerik (bu VM'de `gtk-4.0/gtk.css` matugen çıktısı + Darkly
kurulum artığı): içerik silinmez; `@import` sona eklendiği için Nuxut renkleri kazanır.
Kullanıcı CSS'i de `:root` değişkeni tanımlıyorsa çakışma `plan`'da uyarı olarak görünür
(§9). `gtk.css` symlink ise hedefine yazılır, salt okunursa `ExternallyManaged`.

GTK2 (`~/.gtkrc-2.0`) yönetilmez. Flatpak uygulamaları: portal üzerinden `color-scheme`
alırlar; tema/CSS için `flatpak override` gerekir — kapsam dışı (§12-6).

### 5.3 Qt

**Birincil yol (önerilen): KDE platform teması.**

| Hedef | Tür | İçerik |
|---|---|---|
| `~/.config/kdeglobals` | `IniKey` (yalnızca §5.1'deki `[Colors:*]`, `[ColorEffects:*]` anahtarları + `[General] ColorScheme`, `[General] font/fixed/menuFont/smallestReadableFont/toolBarFont`, `[Icons] Theme`, `[KDE] widgetStyle`) | noctalia'nın birleştirme yöntemi: başka anahtar/bölüm korunur. Renk şeması ayrıca `~/.local/share/color-schemes/Nuxut.colors` (`ManagedFile`) — KDE Sistem Ayarları'nda görünsün diye. |
| Ortam | `QT_QPA_PLATFORMTHEME=kde` (Qt5 için `plasma5-integration` aynı adı kullanır), `QT_STYLE_OVERRIDE=` (boş = yok; Kvantum/Darkly zorlamasını kaldırır) | §5.6 |

Canlı: `kdeglobals` yazıldıktan sonra D-Bus sinyali `/KGlobalSettings
org.kde.KGlobalSettings.notifyChange(int32 type, int32 arg)` — `0` palet, `1` font, `2` stil;
ikon için `/KIconLoader org.kde.KIconLoader.iconChanged(int32 0)` (B6; ikon/font sinyalleri
uygulama adımında doğrulanacak).

Stil: `widgetStyle=Breeze` (extra deposu; paleti izler, KDE uygulamalarıyla aynı). Darkly
(AUR, end4'ün seçimi, daha yuvarlak) seçilebilir; kurulu değilse önerilmez.

**Yedek yol: qt6ct/qt5ct** (plasma-integration kurulamazsa ya da kullanıcı seçerse):
`~/.config/qt6ct/colors/nuxut.conf` + `qt5ct/colors/nuxut.conf` (`ManagedFile`, QPalette
sırası §5.1), `qt6ct.conf`/`qt5ct.conf` `IniKey` `[Appearance] custom_palette=true`,
`color_scheme_path`, `icon_theme`, `style`; `[Fonts] general/fixed`. Ortam
`QT_QPA_PLATFORMTHEME=qt6ct` (Qt5 `qt5ct`). **Canlı değil** (B5): Qt uygulamaları yeniden
başlatılınca alır. KDE uygulamaları renkleri yine `kdeglobals`'tan okur → `kdeglobals` her iki
yolda da yazılır.

**Kvantum yönetilmez.** SVG temaları paleti izlemez; Nuxut renkleriyle tutarlı olamaz.
Mevcut `QT_STYLE_OVERRIDE=kvantum` çakışma olarak raporlanır ve birleşik görünümde boşaltılır
(geri alınabilir). `~/.config/Kvantum/` dosyalarına dokunulmaz.

### 5.4 İmleç

| Hedef | Tür | Canlı |
|---|---|---|
| Hyprland | `hyprctl setcursor <tema> <boy>` eşdeğeri (shell'in Hyprland soketi, `hypr_config` `HyprControl`) | Anında: compositor imleci ve `wp_cursor_shape_v1` kullanan istemciler (GTK4, Qt6, framework, büyük olasılıkla Chromium/Electron) |
| Hyprland ortamı | `HYPRCURSOR_THEME`, `HYPRCURSOR_SIZE`, `XCURSOR_THEME`, `XCURSOR_SIZE` → `hypr_config`'in ürettiği `nuxut.lua`/`nuxut.conf` (`hl.env`/`env =`) — değerleri bu ADR'nin üreticisi verir, satırı `hypr_config` yazar | Yeni başlatılan süreçler |
| systemd/D-Bus ortamı | aynı dört değişken (§5.6) | Yeni başlatılan süreçler |
| dconf `cursor-theme`, `cursor-size`; `settings.ini` `gtk-cursor-theme-*` | §5.2 | GTK3 kendi imlecini çizer, ayarı canlı izler |
| `~/.icons/default/index.theme` | `IniKey` `[Icon Theme] Inherits` | XWayland/X11 istemcileri ve temasız yedek; X istemcileri yeniden başlatılınca |
| `kdeglobals`/`kcminputrc` | `kcminputrc [Mouse] cursorTheme, cursorSize` (`IniKey`) | Qt5 ve wl_cursor kullanan Qt istemcileri; yeniden başlatma |

Hyprcursor biçimli tema yoksa Hyprland XCursor'a düşer; iki değişken de aynı adı alır.
XWayland imleci: Hyprland XWayland'e kendi temasını açılışta verir; çalışan X istemcileri
yeniden başlatmada günceller.

### 5.5 İkon teması ve yazı tipi

İkon: dconf `icon-theme`, `settings.ini` `gtk-icon-theme-name`, `kdeglobals [Icons] Theme`
(yedek yolda `qt6ct.conf icon_theme`). Papirus için ad kipe göre `Papirus-Dark` /
`Papirus-Light` (panel ikonları kipe göre değişir); kullanıcı açıkça bir varyant seçerse
kip izlenmez. Canlı: GTK3/4 ayarı canlı izler; Qt KDE yolunda `iconChanged` sinyali.
Nuxut uygulamaları (`IconTheme::system()`) settings.ini'yi okur → framework'ün değişimde
yeniden okuması gerekir (§13-5).

Yazı tipi: framework `[typography]` rolleri kaynaktır, ayrı seçim yok.
`ui` → dconf `font-name`, `settings.ini gtk-font-name`, `kdeglobals font/menuFont/toolBarFont`;
`document` → `document-font-name`; `mono` → `monospace-font-name`, `kdeglobals fixed`.
Boy: `typography.size` mantıksal px → pt = px × 0.75 (96 dpi), 0.5'e yuvarlanır;
`smallestReadableFont` = 0.8 ×. **Değişti (2026-10-13, §18):** GTK adı (dconf, `gtk-font-name`)
REFERANS boyda (13 px × rol çarpanı); metin ölçeği GTK'ya `text-scaling-factor` +
`gtk-xft-dpi` ile girer. kdeglobals boyu ölçekli kalır. Ağırlık Pango/Qt adına çevrilir (500 → "Medium").
Canlı: GTK font ayarını canlı izler; KDE yolunda `notifyChange(1, 0)`.

### 5.6 Ortam değişkenleri

Değişkenler: `QT_QPA_PLATFORMTHEME`, `QT_STYLE_OVERRIDE` (boş; `environment.d`'ye yazılmaz, §17-11), `XCURSOR_THEME`,
`XCURSOR_SIZE`, `HYPRCURSOR_THEME`, `HYPRCURSOR_SIZE`; qt6ct yolunda ve metin ölçeği ≠ 1 iken
`QT_FONT_DPI` (§18). `GTK_THEME` **yazılmaz** (canlı kip geçişini kilitler); kullanıcı ortamında
varsa çakışma olarak raporlanır. `QT_SCALE_FACTOR` hiç yazılmaz.

| Hedef | Tür | Ne zaman etkili |
|---|---|---|
| `~/.config/environment.d/99-nuxut-look.conf` | `ManagedFile` | Sonraki oturum (systemd kullanıcı yöneticisi). `99-` öneki eski kozmetik dosyalarını (`10-`, `95-`) ezer; önceki sürümün yönetilen `90-nuxut-look.conf`'u look.unified işleminde kaldırılır (göç, snapshot uyumlu). |
| Hyprland üretilen dosyası | `hypr_config` üreticisine girdi | Hyprland `reload`'dan sonra başlatılanlar |
| systemd kullanıcı yöneticisi | canlı: `org.freedesktop.systemd1.Manager.SetEnvironment` (zbus) | Bundan sonra başlatılan birimler |
| D-Bus etkinleştirme ortamı | canlı: `org.freedesktop.DBus.UpdateActivationEnvironment` (zbus) | D-Bus ile etkinleşen uygulamalar |
| Shell'in başlattığı uygulamalar (başlatıcı) | shell kendi ortamını değiştirmez; başlatırken bu değişkenleri `Command::env` ile verir | Hemen |

## 6. Yönetilen parçalar listesi (`nuxut-apply` `nuxut-apply` için)

`SettingId`'ler: `look.unified` (tümü), `look.mode`, `look.icons`, `look.cursor`,
`look.fonts` (yazı tipleri + metin ölçeği), `look.motion` (GTK animasyonları; §18),
`look.qt_backend`.

- **ManagedFile:** `gtk-4.0/nuxut.css`, `gtk-3.0/nuxut.css`,
  `~/.local/share/color-schemes/Nuxut.colors`, `environment.d/99-nuxut-look.conf`; yedek yolda
  `qt6ct/colors/nuxut.conf`, `qt5ct/colors/nuxut.conf`.
- **Line:** `gtk-4.0/gtk.css` ve `gtk-3.0/gtk.css` içinde `@import url("nuxut.css");`.
- **IniKey:** `gtk-3.0/settings.ini` ve `gtk-4.0/settings.ini` `[Settings]` (8 anahtar;
  `gtk-xft-dpi`, `gtk-enable-animations` §18 ile; GTK 4'te ayrıca `gtk-interface-color-scheme`);
  `kdeglobals` (`[Colors:Window|View|Button|Selection|Tooltip|Complementary|Header]` × 12,
  `[ColorEffects:Inactive|Disabled]`, `[General] ColorScheme, font, fixed, menuFont,
  smallestReadableFont, toolBarFont`, `[Icons] Theme`, `[KDE] widgetStyle`);
  `kcminputrc [Mouse] cursorTheme, cursorSize`; `~/.icons/default/index.theme
  [Icon Theme] Inherits`; yedek yolda `qt6ct.conf`/`qt5ct.conf` `[Appearance]
  custom_palette, color_scheme_path, icon_theme, style` ve `[Fonts] general, fixed`.
- **Dconf:** `/org/gnome/desktop/interface/{color-scheme, gtk-theme, icon-theme, cursor-theme,
  cursor-size, font-name, document-font-name, monospace-font-name, text-scaling-factor,
  enable-animations}`.
- **Dosya dışı, snapshot'sız canlı eylemler** (§7): Hyprland `setcursor`; systemd
  `SetEnvironment`/`UnsetEnvironment`; D-Bus `UpdateActivationEnvironment`;
  `KGlobalSettings.notifyChange`; `KIconLoader.iconChanged`. Geri yüklemeden sonra aynı
  eylemler geri yüklenen değerlerle yeniden çalıştırılır (§8-3).
- **Dokunulmayanlar:** Hyprland ana dosyası (yalnızca `hypr_config`'in tek `source`/`require`
  satırı), `~/.config/Kvantum/`, `~/.local/share/themes/` (eski Nuxut teması dahil),
  `~/.gtkrc-2.0`, `xsettingsd`, kullanıcının `environment.d`'deki diğer dosyaları.

## 7. Uygulama sırası ve canlı sınırlar

Değişimde (shell, `nuxut-hypr-sync` ile aynı thread modeli): üret → `nuxut-apply` →
başarılıysa sırasıyla: (1) dconf (GTK/portal anında), (2) `kdeglobals` sinyalleri,
(3) Hyprland `setcursor`, (4) systemd + D-Bus ortamı, (5) `hypr_config`'e "ortam değişti"
(Hyprland dosyası yeniden üretilir). Aynı değer yeniden yazılmaz, sinyal yayınlanmaz.

| Değişen | Anında | Yeniden başlatma ister |
|---|---|---|
| Koyu/açık | libadwaita/GTK4 (B4), GTK3 tema adı (`adw-gtk3[-dark]`) ve Qt KDE yolu (B6), Nuxut | GTK3 özel renkleri (`gtk-3.0/nuxut.css`), qt6ct yolu |
| Renk/palet (duvar kağıdı) | Qt KDE yolu, Nuxut, Hyprland kenarlık | GTK3 ve GTK4 renkleri (B3), qt6ct yolu |
| İkon teması | GTK3/4, Qt KDE yolu (sinyal), Nuxut (§13-5 ile) | qt6ct yolu |
| İmleç | Hyprland ve cursor-shape istemcileri, GTK3 | XWayland/X11, Qt5, ortamı açılışta okuyanlar |
| Yazı tipi | GTK3/4, Qt KDE yolu | qt6ct yolu |
| Qt yolu (kde ↔ qt6ct) | — | Tüm Qt uygulamaları (ortam) |

Ayarlar her değişiklikten sonra kısa bir not gösterir: "Bazı açık uygulamalar yeni görünümü
yeniden açılınca alır." Hangi uygulamaların etkilendiğini tespit etmeye çalışılmaz (v1).

GTK renklerini canlı yapmanın iki yolu sonraya bırakıldı (§12-5): (a) kendi
`org.freedesktop.impl.portal.Settings` arka ucumuz `accent-color`'ı RGB olarak verir —
libadwaita vurgu rengi canlı olur; (b) GTK3 için tema dizini A/B çevirme (B7 sonuçsuz).

## 8. Geri alma ve kullanıcının ayarları

1. "Birleşik görünüm" açılmadıkça hiçbir dış hedef yazılmaz (yeni kurulumda Hoş geldin'de
   önerilen = açık). Kapalıyken shell yalnızca kendi çizimini temaya göre yapar.
2. İlk açılış `look.unified` snapshot'ı alır: yukarıdaki her parçanın Nuxut öncesi değeri
   (yabancı yönetilen dosyalar tümüyle). Bu ilk hâl sabitlenir (§11).
3. **Kapatma = geri yükleme:** `look.unified` kapatılınca ilk snapshot geri yüklenir ve canlı
   eylemler geri yüklenen değerlerle tekrarlanır (eski imleç `setcursor` edilir, ortam
   değişkenleri eski değere/`UnsetEnvironment`'a döner, KDE sinyali).
4. Tema oynadıkça yeniden üretim snapshot üretmez (§6 kuralı); `look.icons`,
   `look.cursor`, `look.mode`, `look.fonts` seçimleri ayar başına "önceki hale dön" verir.
5. Kullanıcı yönetilen bir anahtarı elle değiştirirse (ör. dconf `icon-theme`) Nuxut bir
   sonraki uygulamada onu ezer **ama** önce `plan` farkı görür ve Ayarlar "Dışarıdan
   değiştirildi: Nuxut'a al / Nuxut'tan ayır" sunar (v2; v1'de ezer ve loglar).

## 9. Hoş geldin penceresi (içerik ve akış; UI Ayarlar'da)

Kural: en fazla 3 adım, her adım tek karar, "Atla" her zaman var, teknik ad (GTK, Qt,
Hyprland, dconf) **görünmez** — yalnızca "Ayrıntılar" açılınca.

**0. Karşılama** — "Nuxut'a hoş geldin. Birkaç tercih yapalım; hepsini sonra Ayarlar'dan
değiştirebilirsin." [Başla]

**1. Uygulamalar Nuxut gibi görünsün mü?** (iki büyük seçenek kartı, önizlemeli)
- **Evet, her şey uyumlu olsun (önerilen)** — "Uygulamalar Nuxut'un renklerini, ikonlarını,
  imlecini ve yazı tiplerini kullanır. Mevcut görünüm ayarların değişir; istediğin an tek
  tıkla geri alabilirsin."
- **Hayır, kendi ayarlarımı koru** — "Yalnızca Nuxut'un kendi parçaları temayı kullanır."
- "Ayrıntılar" (kapalı): `plan` çıktısından "Değişecekler" listesi (ör. "Uygulama teması:
  Adwaita → Nuxut", "Qt görünümü: Kvantum → Nuxut", "İmleç: Vimix → Bibata") ve **çakışmalar**
  ("Başka bir araç da görünüm ayarlıyor: eski Nuxut kozmetiği, Hyprland `env` satırları —
  birleşik görünüm bunları geçersiz kılar").
- Eksik paket varsa (ör. plasma-integration): "Bazı uygulamalar için ek bileşen gerekli" +
  kurulum komutu kopyalanabilir; Nuxut paket kurmaz.

**2. Görünüm** (tek sayfa, üç satır; adım 1'de "Hayır" dendiyse yalnızca Nuxut için)
- Koyu / Açık (iki küçük önizleme).
- İkonlar: 3 kart (önerilen Papirus, Adwaita, kurulu diğerleri "Daha fazla…"). Her kartta
  aynı 6 uygulama ikonu (dosyalar, tarayıcı, terminal, ayarlar, metin, müzik).
- İmleç: 3 kart (Bibata, Adwaita, kurulu diğerleri) + boy (küçük/orta/büyük = 24/32/48).
- Seçim anında uygulanır (canlı önizleme gerçek masaüstünde); "Geri" seçimi geri alır.

**3. Masaüstüyle ortak** — "Köşeler, gölgeler, bulanıklık, hareketler ve kısayollar pencerelerle
ortak olsun (önerilen)." Tek anahtar; ADR-0001 §10 adım 3 ve `hypr_config` akışı.

**Bitiş** — "Hazır. Bazı açık uygulamalar yeni görünümü yeniden açılınca alır."
[Bitir] → `setup.toml done = <sürüm>`.

Shell'den gereken bilgiler (`nuxut-shell msg look status`, JSON; Ayarlar Hoş geldin ve
Görünüm sayfası okur):

```json
{ "unified": true, "mode": "dark",
  "icons":   { "current": "Papirus-Dark", "installed": ["Papirus", "Adwaita", "breeze", …] },
  "cursors": { "current": "Bibata-Modern-Classic", "size": 24,
               "installed": [{"name": "Bibata-Modern-Classic", "hyprcursor": false}, …] },
  "qt_backend": { "active": "kde", "available": ["kde", "qt6ct"] },
  "missing_packages": ["plasma-integration"],
  "conflicts": [ {"kind": "env", "name": "QT_STYLE_OVERRIDE", "value": "kvantum",
                  "source": "hypr:configs/default/env.conf:6"},
                 {"kind": "env_file", "source": "environment.d/10-nuxut-cosmetics.conf"},
                 {"kind": "hypr_setcursor", "source": "hyprland.conf:31"} ],
  "restart_needed": ["gtk3_colors", "gtk4_colors"],
  "last_apply": {"ok": true, "skipped": [], "errors": []} }
```

Kurulu tema listesi framework tarayıcısından gelir (§13-4); çakışmalar `hypr_config` zincir
taraması (`env`/`setcursor` satırları), `environment.d` dizini ve shell'in kendi ortamından.
Değerler kullanıcı içeriği sayılmaz (tema adları, yol + satır numarası); loglara yazılabilir.
Yol/değer boyları sınırlı (yol ≤ 512, değer ≤ 256 bayt, liste ≤ 256 öğe).

## 10. matugen ve bağımlılıklar

**matugen gerekmez.** Renk kaynağı framework teması; duvar kağıdı paleti zaten
`nuxut-palette` (material-colors crate'i, matugen'in kullandığı algoritmanın saf Rust portu)
ile üretiliyor ve şablon motoru var. Ayrı ikili, şablon dosyaları ve Python'lu
`kde-material-you-colors` (end4) yerine tek Rust üreticisi.

Shell'e yeni crate yok: D-Bus sinyalleri ve systemd/D-Bus ortam çağrıları mevcut `zbus` ile;
Hyprland `setcursor` mevcut Hyprland soket istemcisiyle; dconf ve dosyalar `nuxut-apply` ile.

`docs/DEPENDENCIES.md`'ye eklenecek çalışma zamanı paketleri (opsiyonel bölüm; yoksa ilgili
hedef `Unavailable`, görünüm o araç için eski hâlinde kalır):

| Paket (Arch) | Depo | Lisans | Neden | Yoksa |
|---|---|---|---|---|
| `adw-gtk-theme` | extra | LGPL-2.1 | GTK3 uygulamalarına libadwaita görünümü + `@define-color` adları | GTK3 Adwaita'da kalır, renkler kısmen uyar |
| `xdg-desktop-portal-gtk` | extra | LGPL-2.1 | Portal `Settings` arka ucu: libadwaita `color-scheme`'i buradan alır | libadwaita kip değişimini görmez |
| `plasma-integration` (Qt6), `plasma5-integration` (Qt5) | extra | LGPL-2.0+ | Canlı Qt paleti/font/ikon (`QT_QPA_PLATFORMTHEME=kde`); KF6 kütüphanelerini (kio vb.) ve `xdg-desktop-portal-kde`'yi çeker — Plasma masaüstü kurulmaz | qt6ct/qt5ct yedeği (yeniden başlatma) |
| `breeze` (Qt6 stil), `breeze5` | extra | LGPL-2.0+ | Qt stili; paleti izler, KDE uygulamalarıyla tutarlı | Fusion (yerleşik, sade) |
| `qt6ct`, `qt5ct` | extra | BSD-2 | Yalnızca yedek Qt yolu | — |
| `papirus-icon-theme` | extra | GPL-3.0 | Varsayılan ikon teması (§11'deki aday tablosu) | Adwaita |
| `bibata-cursor-theme-bin` | AUR | GPL-3.0+ | Varsayılan imleç | `adwaita-cursors` (extra) |

`install.sh`: `PKG_LOOK="adw-gtk-theme xdg-desktop-portal-gtk plasma-integration
plasma5-integration breeze papirus-icon-theme"` pacman listesine; `bibata-cursor-theme-bin`
mevcut `PKG_AUR` akışına (AUR yardımcısı yoksa uyarı + Adwaita yedeği). `--minimal` seçeneği
plasma-integration yerine `qt6ct qt5ct` kurar. install.sh hiçbir görünüm dosyası yazmaz;
ilk açılışta Hoş geldin karar verir.

## 11. Varsayılan ikon ve imleç adayları

Ölçütler: depoda olması, lisans, kapsam (uygulama ikonu sayısı), koyu/açık varyant,
Nuxut görsel dili (DESIGN.md: sakin yüzeyler, yuvarlak geometri, Material Symbols Rounded
ikonlar, tek vurgu rengi).

**İkon teması**

| Aday | Paket | Lisans | Kapsam | Koyu/açık | Uyum |
|---|---|---|---|---|---|
| **Papirus** | `papirus-icon-theme` (extra, 112 MiB) | GPL-3.0 | Çok geniş (binlerce uygulama + simge), sürekli bakımlı | `Papirus`, `-Dark`, `-Light` (panel/simge renkleri kipe göre) | Düz, yuvarlak köşeli, Material'e yakın; `papirus-folders` (AUR, MIT) ile klasör rengi vurguya yaklaştırılabilir (v2) |
| Adwaita | `adwaita-icon-theme` (extra, kurulu) | CC-BY-SA-3.0 / LGPL-3.0 | Simgeler tam; uygulama ikonları uygulamaların kendi hicolor ikonlarına kalır (karışık stil) | Simgeler renk izler | GNOME'un kendi dili; nötr ama tutarsız uygulama ikonları |
| Tela Circle | `tela-circle-icon-theme-all` (extra) | GPL-3.0+ | Geniş | Koyu/açık + 12 renk varyantı | Daire içinde ikonlar — Nuxut'un kare-yuvarlak dilinden ayrışır |

Öneri: **Papirus** (kipe göre `Papirus-Dark`/`Papirus-Light`), yoksa Adwaita.

**İmleç**

| Aday | Paket | Lisans | Nitelik | Koyu/açık | Uyum |
|---|---|---|---|---|---|
| **Bibata Modern** | `bibata-cursor-theme-bin` (AUR, 34 oy) | GPL-3.0+ | Yuvarlak uçlu, keskin HiDPI boyları (24–96), end4 varsayılanı; bu VM'de `Bibata-Modern-Classic` kurulu | `-Classic` (siyah), `-Ice` (beyaz) | Yumuşak, modern; Nuxut'a en yakın |
| Adwaita | `adwaita-cursors` (extra, kurulu) | CC-BY-SA-3.0 / LGPL-3.0 | GNOME 47+ yeniden çizilmiş, yuvarlak | tek | Depoda, sağlam yedek |
| phinger | `phinger-cursors` (AUR) | CC-BY-SA-4.0 | Sade, temiz; açık/koyu | iki | İyi ama AUR ve daha az bilinen |

Öneri: **Bibata-Modern-Classic, 24**; AUR yoksa **Adwaita**. İmleç koyu/açık kipi izlemez
(imleç her zeminde okunmalı; kullanıcı `-Ice`'ı seçebilir).

Qt stili: **Breeze** (extra), Darkly (AUR) seçenek. GTK3: **adw-gtk3**.

## 12. Kararlar (2026-10-10)

Önerilerin hepsi kabul edildi; uygulama ayrıntısı §17.

1. **Dış hedefleri yalnızca shell uygular** (A). Ayarlar yalnızca `framework.toml` yazar,
   önizleme/plan/geri yüklemeyi shell'den (`msg look …`) okur. Dış yazımlar yalnızca shell'den,
   `nuxut-apply` üzerinden; `nuxut-apply` gelene kadar shell'in kendi yazıcı arayüzünün arkasında
   ve **varsayılan olarak hiçbir şey yazılmaz** (§17-3).
2. **Qt yolu:** KDE platform teması + Breeze (A); qt6ct/qt5ct yalnızca yedek (C biçiminde:
   `qt_backend = "auto"` → plasma-integration kuruluysa `kde`, değilse qt6ct). **Kvantum yok.**
3. **Seçimlerin yeri:** `framework.toml [desktop]` + `[colors] mode` (A). Anahtarlar §17-1.
4. Koyu/açık "otomatik" v1'de yok.
5. **GTK renkleri v1'de yeniden başlatma ister**; Ayarlar bunu dürüstçe söyler
   ("Bazı açık uygulamalar yeni görünümü yeniden açılınca alır"). GTK3 için ayrıca: GTK3
   uygulamaları renkleri yalnızca açılışta okur. Kendi portal arka ucu v2.
6. Flatpak v1 dışı.
7. Köşe/biçim GTK'ya v2; v1 yalnızca renk, ikon, imleç, yazı tipi.
8. Eski Nuxut kozmetiği: çakışma olarak gösterilir, dosyalara dokunulmaz (`90-` öneki ezer).
9. **Paletten türetme framework'te** (`nuxut-palette`; matugen yok). Shell rol kümesini
   `nuxut_palette::Palette::generate` ile alır; framework'te eksik olan (tam rol kümesi API'si,
   açık kip, `success`/`warning`) shell'de kopyalanmaz, §13'te talep.
10. **Varsayılan ikon Papirus (öneri, onaysız); imleç Vimix-cursors — kullanıcı kararı 2026-10-11 (Bibata yerine).**
    Tek sabit noktadan değişir (`services/look/selection.rs` `RECOMMENDED_*`); kurulu değilse
    zararsız yedek Adwaita.

## 13. Framework talepleri

1. **Tam rol kümesi her kaynakta:** statik temada da (duvar kağıdı yokken) GTK/Qt için gereken
   Material rolleri (`surface_container_low/high/highest`, `surface_dim`, `outline_variant`,
   `primary_container`/`on_primary_container`, `secondary*`, `tertiary*`, `error_container`/
   `on_error_container`, `inverse_*`). Öneri: statik temada `primary`'yi tohum alıp
   `nuxut-palette` ile türet, sonra `framework.toml`'da açıkça yazılmış 11 token'ı üstüne koy.
   API: `Theme::export_palette(Mode) -> ExportPalette` (iki kip de; rol adı → `Color`).
2. **`success` ve `warning` rolleri** (Material'de yok; Nuxut türetimi ile) — yoksa GTK/KDE
   karşılıkları yazılmaz.
3. **Koyu/açık kip birinci sınıf tema özelliği:** `[colors] mode = "dark" | "light"`, statik
   varsayılan paletin açık karşılığı, `Theme` iki kipi de çözebilsin (GTK4 dosyası iki kipi
   aynı anda taşır, B4). **Geldi** (fw d30fd1b, 34f21a7; `auto` varsayılan, §17-7).
4. **Kurulu tema tarayıcıları:** `nuxut_image::icon::installed_icon_themes()` ve
   `installed_cursor_themes()` (XDG data dirs + `~/.icons`; `index.theme` adı, `Hidden`,
   imleç için `cursors/` ya da `hyprcursor` manifesti; sınırlı: ≤ 512 tema, `index.theme`
   ≤ 64 KiB).
5. **`IconTheme::system()` değişimi izlesin** (settings.ini/dconf değişince yeniden çözülsün) ve
   framework'ün xcursor yedeği `XCURSOR_THEME` dışında seçili temayı da alabilsin.
6. ~~**Saf üreticiler** framework'te~~ — **düştü** (§17-6: üreticiler shell'de kalır). Yalnızca şu kalır:
   mevcut `nuxut-palette/templates/gtk4.css` `@define-color` yazıyor → libadwaita 1.9'da etkisiz (B1);
   düzeltilmeli. Eski öneri, kayıt için:
   `look::gtk4_css(&ExportPalette ×2) -> String` (CSS değişkenleri, B1),
   `look::gtk3_css(&ExportPalette) -> String`, `look::kdeglobals(&ExportPalette, &Typography,
   icon) -> Vec<IniKey>`, `look::qtct_scheme(&ExportPalette) -> String`,
   `look::settings_ini(...)`, `look::dconf(...)`, `look::env(...)`; ve
   `look::unified(&Theme, &DesktopSelection) -> Vec<(Fragment, FragValue)>`.
7. ~~Yazı tipi boyu dönüşümü framework'te~~ — **düştü**: framework'te eşdeğeri yok, tek tüketici shell
   (`services/look/fonts.rs`).
8. Seçimlerin yeri §12-3'e göre `framework.toml` şemasına `[desktop]` (Ayarlar şeması da). **Geldi** (fw d30fd1b, §17-1).

## 14. `nuxut-apply` (`nuxut-apply`) katmanından beklentiler

1. `Fragment::Line` için **dosya sonuna ekleme** ve "yorumlanmış satır var → ekleme" kuralı
   (`@import url("nuxut.css");` için; `append_source_line` mantığı).
2. INI adaptörü: KDE `[Bölüm][Alt]` başlıkları (`[Colors:Header][Inactive]`), `[$e]` ekli
   anahtarlar, GTK `settings.ini`, `index.theme`; yorum/sıra korunur; aynı dosyaya bir
   işlemde çok anahtar tek yazım.
3. `Dconf` adaptörü test örneğinde yazmaz (`Skipped`) — bu ADR'nin canlı eylemleri de aynı
   kapıdan geçer (`Env::guard`): test örneği `setcursor`/systemd/D-Bus ortamı/KDE sinyali
   göndermez.
4. `plan(change)` → her parça için `{before, after, foreign: bool}`; Hoş geldin'in
   "Değişecekler" ve "çakışmalar" listesi buradan.
5. `restore(snapshot)` sonrası çağırana geri yüklenen değerler dönsün (canlı eylemleri
   tekrarlamak için, §8-3).
6. Türetilmiş yeniden üretim (palet değişimi) snapshot üretmesin; ama yönetilen dosyada
   dışarıdan değişiklik varsa (`Expect` uyuşmazlığı) `Changed` dönsün ve shell loglasın.
7. Yazım sonrası "değişmedi" bilgisi (`NoOp`) — sinyaller ve `setcursor` yalnızca gerçekten
   değişince gönderilir.

## 15. Test stratejisi

- Üreticiler saf: altın dosya testleri (`gtk4_css` iki kip, `kdeglobals` anahtar listesi,
  QPalette sırası 22 rol), sabit renk yok bekçisi (yalnızca `ExportPalette`'ten).
- Birim testleri geçici `HOME`/`XDG_*`; dconf yok (`nuxut-apply` sahte adaptörü).
- Canlı testler yalnızca izole sway + `dbus-run-session`; **XDG dizinleri D-Bus oturumundan
  önce** dışa aktarılır, iç betik `XDG_CONFIG_HOME`'un geçici olduğunu doğrulamadan
  `gsettings`/`dconf` çağırmaz (§3 tuzağı). Kontrol listesi: B1–B6 tekrarı + imleç
  (`grim -c` ile kendi çıkışında) + ikon/font sinyalleri.
- Kullanıcının Hyprland'inde yalnızca `setcursor` (bu VM için izin var), kısa, eski değere
  geri dönülerek.

## 16. Uygulama adımları

0. Doğrulamalar: KDE `notifyChange(1)`/`iconChanged` canlılığı; Qt6 `wp_cursor_shape_v1`;
   Chromium/Electron imleci; GTK3 tema A/B (B7) — izole.
1. framework: §13-1/3 (rol kümesi + kip), §13-6 üreticiler + altın testler, §13-4 tarayıcılar.
2. `nuxut-apply`: `nuxut-apply` adaptörleri (§14).
3. shell: `services/look` — izleyici, üretici çağrısı, canlı eylemler (`setcursor`, systemd/D-Bus
   ortamı, KDE sinyalleri), çakışma taraması, `msg look status|apply|restore`; `hypr_config`
   üreticisine imleç ortamı girdisi.
4. settings: Görünüm sayfası + Hoş geldin (§9).
5. install.sh paketleri + DEPENDENCIES.md (§10).

## 17. Uygulama ayrıntısı (2026-10-10)

1. **`framework.toml` anahtarları** — şema, ayrıştırma ve doğrulama framework'te (fw d30fd1b,
   34f21a7: `nuxut::theme::{DesktopConfig, Desktop, QtBackend, Choice, valid_theme_name}`,
   `ColorsConfig::resolve_mode`); shell'de kopya ayrıştırıcı yok. `services/look/selection.rs`
   `nuxut_config::load::<FrameworkConfig>` ile okur, `Theme::from_config`'in doğruladığı `Desktop`'u
   ve yalnızca `desktop.*`/`colors.mode` uyarılarını alır. Geçersiz ad/seçenek → framework
   varsayılanı + uyarı (`Choice`: dosyayı bozmaz; `cursor_size` sınırlanır); okunamaz/bozuk dosya
   (yanlış tür dahil) = birleşik görünüm kapalı.

   ```toml
   [colors]
   mode = "auto"            # auto | dark | light (varsayılan auto, §17-7)

   [desktop]
   unified = false          # birleşik görünüm; false iken hiçbir dış hedef yazılmaz
   icon_theme = ""          # "" = önerilen (Papirus, kipe göre -Dark/-Light); ad ≤ 128 bayt, [A-Za-z0-9 -_.+]
   cursor_theme = ""        # "" = önerilen (Vimix-cursors)
   cursor_size = 24         # 16..=96 (dışı sınırlanır)
   qt_backend = "auto"      # auto | kde | qt6ct ("qtct" eski ad olarak kabul)
   ```
2. **Plan** (`services/look/plan.rs`): ayar başına `LookChange { setting, label, writes: [(Fragment,
   değer)] }`; `Fragment` = `nuxut-apply` modeliyle birebir (`ManagedFile`, `Line` dosya sonu,
   `IniKey`, `Dconf`). Canlı eylemler yalnızca bağlı ayar gerçekten yazıldıysa çalışır.
3. **Yazım kapısı** `services/look/backend.rs` `LookBackend`; `nuxut-apply` framework main'e inene
   kadar `Disabled` — hiçbir dosya, dconf, sinyal, `setcursor` ya da ortam yazılmaz. **Geçti:** gerçek
   arka uç §17-11.
4. Sapmalar: `DecorationHover` tüm gruplarda `primary` (Breeze geleneği; `outline_variant` Breeze'de
   görünmüyordu). qt6ct/qt5ct yedek yolunda yazı tipleri v1'de yazılmaz (qt5ct `@Variant` ikili
   biçimi). qt6ct yolunda `QT_QPA_PLATFORMTHEME=qt6ct` Qt5 uygulamalarını kapsamaz (bilinen sınır).
5. `msg look status` sözleşmesi (sürüm 2) §9'daki örneğin genişletilmişi: `icons.installed`
   `[{id, name, hidden}]`, `cursors.installed` `[{id, name, xcursor, hyprcursor}]` (framework
   `nuxut_image::icon::installed_*`), `missing_packages` `[{name, source: "repo"|"aur"}]`.
   Sürüm 3 (2026-10-12, istek): `missing_packages[].target`
   (`icons|cursors|gtk|portal|qt`); `icons.catalog` / `cursors.catalog`
   `[{family, name, package, source, installed: [id…]}]` — kurulabilir aileler (`look::catalog`;
   imleçler install.sh `--with-cursors` seti + Bibata, ikonlar Papirus, Breeze, Tela Circle,
   Colloid, Qogir, Kora, Fluent, WhiteSur), kurulu varyantlar dizin adının küçük harfli önekiyle.
   Sürüm 4 (2026-10-12): `themes_rev` (u64) — ikon/imleç/GTK tema dizinlerinde tema eklenince,
   silinince ya da `index.theme` değişince artan sayaç (`look::watch`, inotify; süreç başına 0'dan,
   izleme kapalıysa 0'da kalır). Ayarlar Görünüm ızgarasını değer farklıysa yeniden okur.
   Hyprland zincirindeki `env`/`setcursor` çakışmaları `hypr_config` taramasına eklenene kadar yalnızca
   shell'in süreç ortamı (`session_env`) ve `environment.d` taranır.
6. **Üreticilerin yeri (karar, 2026-10-11):** GTK/Qt/env/yazı tipi üreticileri shell'de
   (`services/look`) kalır — dış yazımları yalnızca shell yapar, tek tüketici shell. §13-6 ve §13-7
   talepleri düştü; framework'ten yalnızca rol kümesi/kip (§13-1..3), tarayıcılar (§13-4), §13-5, §13-8
   ve `nuxut-palette`'in görsel çözmeyi varsayılan açık bir `image` özelliği arkasına alması istenir
   (shell `default-features = false` kullanır; bildirim görsel çözücüsüne JXL girmemeli).
7. **Kip çözümü framework'te:** shell kipi kendisi çözmez; runtime temasının `Theme::mode`'unu
   kullanır (fw 34f21a7 `ColorsConfig::resolve_mode`: `dark`/`light` olduğu gibi; `auto` — varsayılan
   — `source = "wallpaper"` iken `palette.toml` kayıtlı kipi (`GeneratedPalette::recorded_mode`) →
   statik yüzeyin parlaklığı (`ThemeMode::of_surface`, WCAG eşiği; açıkça yazılmış `surface` dahil)
   → koyu). Böylece GTK/Qt, Nuxut'un o an çizdiği kiple aynıdır. `Theme::colors` zaten o kipin
   paletidir; temanın 11 token'ı çözülmüş kipe yazılır. Tek çelişki: sabit kip + ters parlaklıkta
   açıkça yazılmış `colors.surface` (ör. `mode = "dark"`, açık yüzey) — token'lar yazılmaz, o kip
   türetilmiş rollerle kalır ve `msg look status` `warnings` uyarır.
8. Kullanıcı dosyalarına INI birleştirme, `@import` satırı ekleme `nuxut-apply` adaptörlerinin işidir;
   shell'de kopyası yok (yalnızca baştan üretilen `Nuxut.colors` için düz biçimleyici).
9. **Framework'ten gelenler (fw 6d2d0fc):** rol kümesi `nuxut_palette::Scheme` (57 rol, `success`/
   `warning` dahil; `Palette::scheme(Mode)`), GTK CSS `nuxut_palette::look::{gtk4_css, gtk3_css}`,
   tema tarayıcıları. kdeglobals `ForegroundPositive`/`ForegroundNeutral` artık `success`/`warning`'den
   yazılır (grup başına 12 anahtar).
10. **Framework'ten gelenler (fw d30fd1b, 34f21a7, 5c912ce, 9300dd1):** `[desktop]` + `[colors] mode`
    gerçek şema ve `Theme::{mode, desktop}` (§17-1, §17-7; shell `QtBackend` ve `DesktopSelection`
    kopyaları silindi, `qt6ct` değeri `QtBackend::Qt6ct`); `nuxut-palette` `image` özelliği (shell
    `default-features = false`; `cargo tree -i jxl-oxide` shell ağacında eşleşmez); GTK 3/4 CSS'in
    ilk satırı yönetilen dosya başlığıdır (`/* Nuxut — birleşik görünüm, GTK … · elle düzenleme bir
    sonraki uygulamada ezilir */`, `nuxut-apply` `edit::managed::is_managed` ile tanınır) — shell
    ayrıca başlık eklemez.
11. **Yazım arka ucu (2026-10-11, §17-3'ün yerine):** `services/look/writer.rs` `ApplyBackend` —
    `LookChange` → `nuxut_apply::Change` (kaynak `shell`, ayar başına tek işlem; bölünmez). Kapı:
    `Env::guard`; test örneğinde dconf parçaları düşülür, dosyalar yalnızca test köklerine; canlı
    eylemler `live_gate` (test/ikincil örnek: hiçbiri; `setcursor` ayrıca Hyprland oturumu ister).
    Canlı eylemler (§7, hepsi §12'de onaylı): KDE `notifyChange`/`iconChanged` (zbus sinyali),
    Hyprland `setcursor` (shell'in Hyprland soketi), systemd `SetEnvironment` + D-Bus
    `UpdateActivationEnvironment` (zbus, 2 sn zaman aşımı); yalnızca bağlı ayar gerçekten yazıldıysa.
    Çalışma anı `app/look.rs` (`nuxut-look` thread'i, 120 ms birleştirme, yeni istek turu keser, aynı girdi gönderilmez).
    Sapmalar ve sınırlar:
    - **Kapatma (§8-3):** `look.*` ayarlarının sabitlenmiş işlemleri yeniden eskiye `Force` ile geri
      yüklenir (en eskisi en son: her parça Nuxut öncesine; sonradan elle değiştirilmiş yönetilen
      anahtarlar da). Ardından KDE sinyalleri, systemd `UnsetEnvironment` (D-Bus etkinleştirme
      ortamında silme yok), geri yüklemelerin döndürdüğü GTK3 `settings.ini` imleciyle `setcursor`
      (`restore_with_values`; son geri yükleme kazanır, disk ayrıca okunmaz). "Etkin" = bir `look.*`
      ayarının en yeni işlemi uygulanmış ve geri yükleme değil (yeniden başlatmada da doğru).
    - **`gtk.css`** her zaman `Line` (`Change::write_creating`, fw a35725b): yoksa yalnızca bizim
      satırımızla yaratılır, geri yüklemede başka içerik kalmamışsa silinir. Önceki sürümün başlıklı
      yarattığı dosya diskte olağan `Line` olarak işlenir (eski snapshot'ı ise o dosyayı kapatmada
      siler — başlığın vaadi). kdeglobals'ta Nuxut'un yarattığı bölümler (`section_created`) boş
      kalınca başlığıyla silinir: kapatma byte-eşit.
    - **`look.mode` ~105 parça** (kdeglobals 7 renk grubu × 12) tek işlemde, tek snapshot'la
      yazılır (nuxut-apply `MAX_FRAGMENTS = 256` + `MAX_ADDRESS_BYTES` 64 KiB, fw b4e4801).
      Adres bütçesi parça başına yolu sayar: yapılandırma dizini yolu ~600 baytı aşarsa `look.mode`
      bölünmez, `too_many_fragments` ile hiç yazılmaz (diğer ayarlar yazılır). Testli.
    - Görünüm yazımları türetilmiştir: `Applier::apply_derived` — araya kimse girmemişse son
      işleme süre penceresiz birleşir (tema/duvar kâğıdı değişimi yeni snapshot üretmez; ADR-0001
      §6); ilk dokunuşta ya da dış değişiklikte yeni snapshot. Türetilmiş GTK/Qt dosyaları
      `apply_derived` ile look snapshot'ında birleşir; "önceki seçime dön" ihtiyacı framework.toml
      seçiminin kendi snapshot'ıyla karşılanır (o geri alınınca dosyalar yeniden türetilir;
      karar).
    - Tek bir `look.*` `msg snapshot restore` ile geri alınınca KDE sinyallerine ek olarak geri
      yazılan GTK3 imleciyle `setcursor` gönderilir (`restore_with_values` → `after_restore`).
    - İmleç ortamı `hypr_config`'e `SyncRequest::cursor_env` ile (`env = K,V` / `hl.env`; canlı uygulanmaz).
    - **Ayarlar arası geri yükleme sırası serbest** (2026-10-13): Nuxut'un yarattığı ve
      birden çok ayarın paylaştığı dosya (`gtk-4.0/settings.ini`) yaratıcı bilgisini nuxut-apply'da
      dosya başına taşır (`file_inherited`, fw 8139ec1); hangi ayar son geri yüklenirse yüklensin
      boşalan dosya silinir. Önceki geçici LIFO sırası (124df00) kaldırıldı; ayarlar `setting::ALL`
      sırasıyla geri yüklenir.
    - **Boş değerli değişken `environment.d`'ye yazılmaz** (2026-10-13, bulgu):
      systemd `QT_STYLE_OVERRIDE=` satırını geçersiz atama sayıp yok sayar. Bu yüzden §5.6'daki
      "`QT_STYLE_OVERRIDE` (boş)" yalnızca istenen değerdir. Çakışma taramasında boş olmayan her
      değer (Kvantum/Darkly; eski `10-`/`95-` kozmetik dosyaları dahil) çakışma olarak raporlanır.
      Bilinen sınır: `environment.d` başka bir dosyanın koyduğu değişkeni kaldıramaz; kullanıcı o
      satırı kendisi silmelidir (çakışma bunun için gösterilir).

## 18. Erişilebilirlik: karşıtlık, hareket, metin ölçeği (2026-10-13)

Karar: framework t124 sözleşmesi (`../framework/docs/design/accessibility.md` §1–3)
(uygulama). Kaynak her zaman `framework.toml` → `Theme`; look yalnız aktarır.

1. **Yüksek karşıtlık** (`[colors] contrast = "high"`): dışa aktarılan rol kümesi
   `nuxut_palette::Palette::generate_with(tohum, varyant, Dark, mcu_contrast, theme.contrast)`;
   iki kipin 57 rolü de WCAG AA eşiğine itilir (yüzeyler aynı), temanın 11 token'ı (zaten yüksek
   karşıtlıklı) üstüne yazılır. Yalnız `look.mode` değişir. Canlı: `theme_changed` → görünüm turu
   (`app/look.rs` `same_input` artık `contrast`'ı da karşılaştırır).
2. **Azaltılmış hareket** (`Theme::reduced_motion()`): dconf `enable-animations = !reduced_motion`
   ve `gtk-3.0`/`gtk-4.0` `settings.ini` `gtk-enable-animations = 1|0`. Yeni ayar kimliği
   **`look.motion`** (ayrı snapshot: animasyon tercihi tek başına geri alınabilir; kapatmada
   kullanıcının önceki değeri döner). Hareket açıkken `1` yazılır: kullanıcının elle koyduğu `0`
   birleşik görünüm açıkken ezilir (Nuxut "Kapalı" ile ifade edilir), kapatınca geri gelir.
3. **Metin ölçeği** (`f = size/13`, `nuxut_palette::text_scale`): ölçek her hedefe TAM BİR kez.
   - GTK: yazı tipi adı (dconf `font-name`/`document-font-name`/`monospace-font-name`,
     `gtk-font-name`) REFERANS boyda (`gtk_font_pt`); ölçek dconf `text-scaling-factor`
     (`%.2f`, ör. `1.00`, `1.23`) + `settings.ini gtk-xft-dpi` (`round(96·1024·f)`). Her ikisi
     `look.fonts`'ta, ölçek 1'de de yazılır (deterministik; snapshot önceki değeri saklar).
   - Qt kdeglobals yolu değişmedi (`qt_font_pt`, ölçek pt'de).
   - qt6ct yolu: `QT_FONT_DPI = round(96·f)` `environment.d/99-nuxut-look.conf`'a ve oturum
     ortamına (`look.unified`); `f = 1` ya da kde yolunda yazılmaz ve canlı `UnsetEnvironment`
     ile kaldırılır. `QT_SCALE_FACTOR` asla. `QT_FONT_DPI` izlenen değişkenlere eklendi: bizim
     yazmadığımız/farklı bir değer çift ölçek demek olduğundan çakışma olarak raporlanır; kapatma
     onu da `UnsetEnvironment` ile kaldırır.
   - **Göç:** önceki sürüm GTK adına ölçekli pt yazıyordu (`size = 16` → `12`). Sonraki tur adı
     referans boya çevirir; `apply_derived` aynı `look.fonts` işlemine birleşir, kapatma Nuxut
     öncesine byte-eşit döner (test: `legacy_scaled_gtk_font_migrates_to_reference_size`).
4. **Bilinen sınırlar:** `UnsetEnvironment QT_FONT_DPI` kullanıcının kendi koyduğu (Nuxut dışı)
   değeri de o oturum için systemd ortamından kaldırır (çakışma zaten gösterilir; `environment.d`
   bir sonraki oturumda geri yükler). qt6ct yolunda kdeglobals QFont'ları da yazılır (KDE platform
   teması yokken etkisiz). Hareket için Qt'de standart anahtar yok (framework §2).
5. `msg look status` biçimi değişmedi (`STATUS_VERSION` 4). `msg look plan` yeni parçaları ve
   `session_unset` canlı eylemini listeler; `LOOK_SETTINGS` (snapshot kimlikleri) 7 öğe.
6. **Hyprland pencere kenarlıkları** (karar: 2026-10-13; uygulama).
   `hypr_config::generate::WindowsGroup::from_theme`: standart kipte birebir eskisi (odaklı
   `primary`, pasif `outline × 0.4`; test `border_colors_standard_unchanged`). Yüksek karşıtlıkta
   iki kenarlık da `nuxut::ui::style::legible(theme, renk, [surface], NON_TEXT_MIN)` ile ≥ 3:1'e
   (WCAG 1.4.11) en az değişimle itilir: önce opaklık artar, tam opak da yetmezse renk uca
   karışır; ölçüm 8 bit, `surface` üstüne bindirilmiş pikselle yapılır. Bilgi yolu:
   `framework.toml [colors] contrast` → `Theme::contrast` (+ zaten yüksek karşıtlıklı palet) →
   `app/hypr_sync.rs` → `WindowsGroup::from_theme`; `contrast` değişince renkler değiştiğinden
   eşitleme turu kendiliğinden yeniden üretir.
   - **Ölçüm zemini = tema `surface`'i** (koyu/açık). Gerçek zemin bilinemez: kenarlığın bir yanı
     pencere içeriği, öbür yanı boşluktaki duvar kâğıdı ya da komşu pencere. Birleşik görünüm
     açıkken uygulamalar pencere zeminini temanın `surface`'inden alır (GTK/Qt rol eşlemesi,
     §5.1), kenarlığın her pencerede bitiştiği tek bilinen yüzey odur; duvar kâğıdı rastgele
     olduğundan ona karşı hiçbir sabit renk eşik garanti edemez (bilinen sınır).
   - **Odaklı kenarlık** `primary` yüksek paletinde zaten tüm yüzeylere ≥ 4.5 olduğundan
     değişmez (yardımcı eşiği tutan rengi aynen döndürür; test ile doğrulanır).
   - Değerler (varsayılan tema, `hypr_color`, oran `surface`a 8 bit): koyu yüksek pasif
     `rgba(6e727e66)` (1.62) → `rgba(6e727ed5)` (3.07); açık yüksek pasif `rgba(82858e66)`
     (1.55) → `rgba(82858ee8)` (3.07) — yalnız opaklık arttı, ton aynı. Odaklı değişmedi:
     koyu `rgba(9ec5ffff)` (10.42), açık `rgba(3b608fff)` (6.15). Standart pasif (değişmedi):
     koyu `rgba(2c303b66)` (1.12), açık `rgba(c3c6cf66)` (1.20).
