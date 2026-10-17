# Paketleme: Arch (AUR `nuxut-git`) ve crates.io

Karar: 2026-10-16. Dosyalar: `dist/arch/PKGBUILD` (+ `.SRCINFO`), `dist/session/`,
`dist/pam/nuxut-lock`.

## Arch paketi

Tek paket, dört depo (yol bağımlılıkları yan yana klon ister): `framework`, `shell`, `settings`,
`calendar` (`git+https://github.com/nuxut/<repo>.git`). Derleme `--frozen` (kilit dosyaları
depodan), ortak `CARGO_TARGET_DIR` (framework bir kez derlenir). `check()` kurulacak her ikilinin
derlendiğini doğrular; eksik ikili sessizce paketlenmez.

| Yer | İçerik |
|---|---|
| `/usr/bin` | `nuxut-shell`, `nuxut-lock`, `nuxut-polkit`, `nuxut-settings`, `nuxut-calendard` |
| `/usr/lib/nuxut` | `xdg-desktop-portal-nuxut`, `nuxut-filechooser`, `nuxut-sharepicker`, `nuxut-image-worker`, `nuxut-session` (kabuk yardımcıları çalışan ikiliye göreli `../lib/nuxut` altında arar) |
| `/usr/lib/systemd/user` | `nuxut-shell`, `nuxut-polkit`, `xdg-desktop-portal-nuxut`, `nuxut-calendard` birimleri (repo dosyalarındaki ev dizini yolları paket yollarına çevrilir) |
| `/usr/share/dbus-1/services` | portal arka ucu ve `org.nuxut.Calendar1` etkinleştirmesi |
| `/usr/share/xdg-desktop-portal/portals` | `nuxut.portal` |
| `/etc/pam.d/nuxut-lock` | `auth include login` (yoksa kilit `login` servisine düşer ve uyarı loglar) |
| `/usr/share/wayland-sessions/nuxut.desktop` | oturum girdisi (aşağıda) |
| `/usr/share/applications`, `/usr/share/icons` | Ayarlar, Hoş geldin, webcal girdileri; Ayarlar ikonu |
| `/usr/share/licenses/nuxut-git/LICENSE` | MIT |

Bağımlılıklar `docs/DEPENDENCIES.md`'deki Arch adlarıyla: zorunlular `depends`, "yoksa özellik
sessizce kapanır" olanlar `optdepends`. Material Symbols resmî depodaki
`ttf-material-symbols-variable` ile `depends`'te (2026-10-16: framework'ün kullandığı 114 ligatürün
hepsini çözdüğü doğrulandı; AUR `-git` sürümü gerekmez).

Paket kullanıcı dosyalarına yazmaz. Kullanıcıya ait adımlar (Hyprland ayarına `source` satırı,
`xdph.conf` paylaşım seçicisi bloğu, portal seçimi `nuxut-portals.conf`) kabuk ve portal ikilisi
tarafından ilk açılışta, yedekli ve geri alınabilir biçimde yapılır (ADR-0001, ADR-0003).

### Oturum girdisi

`nuxut.desktop` → `/usr/lib/nuxut/nuxut-session`: Hyprland'i (`start-hyprland`, yoksa `Hyprland`)
başlatır. Kabuk açıldığı ilk oturumda Hyprland ayarına kendi kancasını ekler; o kanca yoksa
(`$XDG_STATE_HOME/nuxut/hypr-source-added` işareti yok: paketten ilk giriş) betik Hyprland'in
örneği belirince (`hyprctl instances`, pid betiğin kendisi ya da çocuğu) kabuğu o örneğin
`HYPRLAND_INSTANCE_SIGNATURE` ve `WAYLAND_DISPLAY`'iyle bir kez başlatır. Betik hiçbir ayar
dosyasına yazmaz. Bilinen sınır: gerçek bir giriş yöneticisiyle canlı denenmedi (2026-10-16;
`hyprctl instances` ayrıştırması bu makinede doğrulandı).

## install.sh ile ilişkisi

`install.sh` paketi kurmaz ve AUR yardımcısı çağırmaz: `nuxut-git` (ya da `nuxut`) kuruluysa
yalnız `pacman -Qi` ile algılar, "paket yöneticinle güncelle" deyip çıkar; değilse kaynaktan
(sabit uzak adres, `--ref`/`NUXUT_REF` ile etiket ya da commit sabitleme, `cargo build --locked`) derleyip kurar ve
kurulan her ikiliyi doğrular (güvenlik incelemesi 2026-10-16). Kabuk başlangıç dosyalarına (`~/.bashrc`, `~/.profile`…)
yazmaz; PATH sorunu varsa yalnız çözümü yazdırır ya da açık istekle (`--fix-path`)
`environment.d` dosyası yazar.

## crates.io (alfa)

Sürüm `0.2.0-alpha.1` (crate'ler ve uygulamalar; karar 2026-10-16: 0.1.0'dan büyük, SemVer'de
geri gidiş yok, alfa olduğu belli). Adlar 2026-10-16'da boştu (`nuxut`, `nuxut-ui`, `nuxut-wayland`,
`nuxut-shell` ve aşağıdaki bağımlılıklar). Bir crate yayımlanabilmesi için bütün `nuxut-*`
bağımlılıkları önce yayımlanmış olmalı; yol bağımlılıkları `version` da taşır. Sıra
(bağımlılık önce):

1. framework: `nuxut-core`, `nuxut-config`, `nuxut-locale`, `nuxut-i18n-macros`, `nuxut-portal`,
   `nuxut-net`, `nuxut-motion`, `nuxut-access`, `nuxut-apply`, `nuxut-render`, `nuxut-theme`,
   `nuxut-i18n`, `nuxut-image`, `nuxut-image-sandbox`, `nuxut-palette`, `nuxut-video`,
   `nuxut-ui`, `nuxut-wayland`, `nuxut`
2. calendar: `nuxut-calendar-ipc`
3. shell: `nuxut-dbus`, `nuxut-nm`, `nuxut-bluez`, `nuxut-power`, `nuxut-wallpapers`,
   `xdg-desktop-portal-nuxut`, `nuxut-shell`

Sürüm yükseltmesi ve crate metadata'sı (`authors` = proje sahibi,
`homepage`, İngilizce `description`, `keywords`, `categories`, `readme`, `publish = true`, yol
bağımlılıklarına `version = "0.2.0-alpha.1"`) yayın günü tek commit'le yapılır
(2026-10-16): 0.1.0 → 0.2.0-alpha.1 ileri gidiştir, "güncelleme bitti" bildirimi çalışır; framework
sürümü değişince shell/Ayarlar/takvim `Cargo.lock`'ları birlikte güncellenmeli,
yoksa `--locked` derlemeler kırılır. Yayın günü sırası: dört repoda sürüm + metadata + kilitler
aynı anda iner, ardından yukarıdaki sırayla `cargo publish`.

`nuxut-image-worker`, `nuxut-lock`, `nuxut-polkit`, `nuxut-pam`, Ayarlar ve takvim servisi
crates.io'ya çıkmaz (`publish = false`): ikili/sistem bileşenleri paketle dağıtılır.
crates.io yeni crate adlarında hız sınırı uygular (kısa bir patlamadan sonra dakikalar arayla
bir yeni crate); 27 crate'lik ilk yayın saatlere yayılır. Her adımdan önce
`cargo package -p <crate>` ve `cargo publish --dry-run`; belirteç yalnız
`~/.config/nuxut-team/crates-token` dosyasından, komut satırına girmez.
