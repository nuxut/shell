#!/bin/sh
# Nuxut kurulum betiği — Arch Linux + Hyprland.
#
# Kaynakları (framework, shell, settings, calendar) indirir ya da günceller, derler ve
# kullanıcı dizinine kurar. root gerektirmez; yalnızca eksik paketleri kurarken sudo ister. Kök
# dosyaları (sistem görünümü, giriş ekranı) yalnız istenince, yönetici onayıyla kurulur.
#
#   curl -fsSL https://raw.githubusercontent.com/nuxut/shell/main/install.sh | sh
#   (ya da: git clone https://github.com/nuxut/shell.git && sh shell/install.sh)
#
# Tekrar çalıştırmak güncelleme yapar (kaynakları çeker, yeniden derler, kurar).
set -eu

# --- Kendini güncelleme -----------------------------------------------------------------
# Betik $SRC/shell'i günceller ama çalışan kopya eskisidir: güncellemeden sonra
# $SRC/shell/install.sh değiştiyse aynı argümanlarla bir kez kendini yeniden exec eder
# (istek 2026-10-16; docs/ARCHITECTURE.md "Kurulum"). Bekçi: yeniden exec edilen
# kopya NUXUT_INSTALL_REEXEC=1 ile başlar ve bir daha exec etmez. Değişken hemen silinir,
# kurulumun başlattığı süreçlere (kabuk, birimler) sızmaz.
REEXECED=${NUXUT_INSTALL_REEXEC:-}
unset NUXUT_INSTALL_REEXEC
# Çalışan kopyanın parmak izi; `curl … | sh` gibi dosyasız çalışmada boş (karşılaştırma yapılmaz).
# $0 yalnız install.sh adını taşıyorsa dosyadır: borudan çalışmada $0 "sh"tır ve çalışma dizinindeki
# "sh" adlı bir dosya parmak izi sayılmamalı (güvenlik incelemesi 2026-10-16).
SELF_SUM=""
case $0 in
    *install.sh) [ -f "$0" ] && [ ! -L "$0" ] && SELF_SUM=$(cksum < "$0") ;;
esac
# Özgün argümanlar, ayrıştırma onları tüketmeden önce: her biri tek tırnakla, `eval set --` için.
ORIG_ARGS=""
for _arg in "$@"; do
    ORIG_ARGS="$ORIG_ARGS '$(printf '%s' "$_arg" | sed "s/'/'\\\\''/g")'"
done

# --- Ayarlar (ortam değişkeniyle ya da seçeneklerle değişir) ---------------------------
PREFIX=${NUXUT_PREFIX:-$HOME/.local}
SRC=${NUXUT_SRC:-$HOME/.local/src/nuxut}
GIT_BASE=${NUXUT_GIT_BASE:-}
# Herkese açık depolar; ayna için NUXUT_GIT_BASE (+ depo adları önekliyse NUXUT_REPO_PREFIX).
GIT_HTTPS=https://github.com/nuxut
GIT_SSH=ssh://git@github.com/nuxut
REPO_PREFIX=${NUXUT_REPO_PREFIX:-}
# Ayna hatırlama (2026-10-16): ortamla verilen NUXUT_GIT_BASE/NUXUT_REPO_PREFIX kaynaklar
# çekilince $XDG_STATE_HOME/nuxut/install.conf'a (0600) yazılır; sonraki çalıştırmada ortam
# yoksa oradan okunur ve origin denetimi ona göre yapılır. Özel adres betikte değil, yalnız
# kullanıcının durum dosyasında. Dosya bize ait, sembolik bağ değil, grup/başkaları yazamaz
# olmalı; yalnız `base=` ve `prefix=` satırları okunur (eval yok), değerler aşağıdaki
# denetimlerden yine geçer.
case ${XDG_STATE_HOME:-} in /*) INSTALL_CONF=$XDG_STATE_HOME/nuxut/install.conf ;; *) INSTALL_CONF=$HOME/.local/state/nuxut/install.conf ;; esac
MIRROR_FROM_ENV=0
MIRROR_FROM_CONF=0
[ -n "${NUXUT_GIT_BASE:-}" ] && MIRROR_FROM_ENV=1
if [ "$MIRROR_FROM_ENV" = 0 ] && [ -f "$INSTALL_CONF" ]; then
    _st=$(stat -c '%u %a' "$INSTALL_CONF" 2>/dev/null) || _st=""
    if [ -L "$INSTALL_CONF" ] || [ "${_st%% *}" != "$(id -u)" ]; then
        echo "HATA: $INSTALL_CONF size ait değil ya da sembolik bağ; silin ya da NUXUT_GIT_BASE verin." >&2; exit 1
    fi
    case ${_st#* } in *[2367]?|*[2367]) echo "HATA: $INSTALL_CONF grup/başkaları yazabiliyor (chmod 600)." >&2; exit 1 ;; esac
    while IFS= read -r _l; do
        case $_l in
            base=*) GIT_BASE=${_l#base=} ;;
            prefix=*) [ -n "${NUXUT_REPO_PREFIX+x}" ] || REPO_PREFIX=${_l#prefix=} ;;
        esac
    done < "$INSTALL_CONF"
    MIRROR_FROM_CONF=1
fi
# Ortamdan ya da kayıttan gelen kaynak aynı doğrulamadan geçer (güvenlik incelemesi 2026-10-17:
# untrusted persisted source): taban yalnız https://, ssh:// ya da mutlak yerel yol; boşluk ve
# denetim karakteri yok; önek yalnız [a-z0-9-] ve '-' ile başlamaz (seçenek gibi görünemez).
if [ "$MIRROR_FROM_CONF" = 1 ]; then _src=$INSTALL_CONF; else _src="NUXUT_GIT_BASE/NUXUT_REPO_PREFIX"; fi
case $GIT_BASE in
    *[[:space:][:cntrl:]]*) echo "HATA: kaynak tabanında boşluk ya da denetim karakteri ($_src)" >&2; exit 1 ;;
    ''|https://?*|ssh://?*|/*) ;;
    *) echo "HATA: NUXUT_GIT_BASE yalnız https://, ssh:// ya da mutlak yerel yol olabilir: $GIT_BASE (kaynak: $_src)" >&2; exit 1 ;;
esac
case $REPO_PREFIX in
    -*|*[!a-z0-9-]*) echo "HATA: depo öneki yalnız [a-z0-9-] ve '-' ile başlamaz: $REPO_PREFIX ($_src)" >&2; exit 1 ;;
esac
BRANCH=${NUXUT_BRANCH:-main}
# Sabitleme (supply-chain, 2026-10-16): etiket ya da commit; boşsa dal ucu. Depo başına
# NUXUT_REF_FRAMEWORK|SHELL|SETTINGS|CALENDAR geçersiz kılar. Çekilen commit'ler çıktıya ve
# $XDG_STATE_HOME/nuxut/install-sources kaydına yazılır.
REF=${NUXUT_REF:-}
CARGO=${NUXUT_CARGO:-cargo}
DEPS=1
HYPR=yes        # yes | no (no: shell.toml [compositor] ensure_source = false)
ASSUME_YES=0
FIX_PATH=0      # 1: ~/.config/environment.d/50-nuxut-path.conf da yazılır (Hyprland dışı oturumlar)
DRY_RUN=0
RESTART=1       # 0: başarılı kurulumdan sonra çalışan kabuk yeniden başlatılmaz (--no-restart)
CURSORS=0       # 1: isteğe bağlı imleç teması seti de kurulur (--with-cursors)
SHAREPICKER=1   # 0: --no-sharepicker (xdph.conf'a ekran paylaşım seçicisi bloğu yazılmaz)
FILECHOOSER=1   # 0: --no-filechooser (dosya seçici seçimi devralınmaz; tercih nuxut-portals.conf'ta kalır)
TAKEOVER=1      # 0: --keep-services (çakışan bar/bildirim servisleri devralınmaz, yalnız uyarı)
HYPRLAND_TAKEOVER=0 # 1: --hyprland-takeover (Hyprland ana ayarı devralınır; güncellemede kendiliğinden ASLA)
ACTION=install
# Kurulum ve güncelleme ayrıdır (kullanıcı isteği 2026-10-16). Güncelleme kullanıcının ayarlarını
# KORUR: install.sh ayar dosyalarına dokunmaz; eski biçimi kabuk açılışta sürüm göçüyle taşır, eski/
# geçersiz satırları onarır (önce yedek; geri al: nuxut-shell msg config undo; durum: msg config
# status — docs/design/config-migrations.md). Hoş geldin yalnızca gerçek ilk kurulumda gelir
# (setup.toml yokken). Taze kurulum yalnızca açık istekle (--reset-config): çalışan kabuk kapanır,
# birleşik görünüm Nuxut öncesine döner (look revert), Nuxut'un ayar ve durum dosyaları
# $XDG_STATE_HOME/nuxut/backups/<zaman>/ altına taşınır, Hoş geldin yeniden açılır.
RESET_CONFIG=0  # 1: --reset-config (bu çalıştırmada taze kurulum)
BACKUP_KEEP=5   # en çok bu kadar yedek kalır (en eskisi silinir)

REPOS="framework:${REPO_PREFIX}framework shell:${REPO_PREFIX}shell settings:${REPO_PREFIX}settings calendar:${REPO_PREFIX}calendar"
# nuxut-lock kabuğun YANINDA olmalı: kabuk onu `current_exe` kardeşi olarak başlatır (PATH'e
# güvenmez; src/services/lock_launcher.rs). 2026-10-16: listede yoktu, Kilitle hiçbir şey açmıyordu.
BINS="nuxut-shell nuxut-lock nuxut-polkit nuxut-settings nuxut-calendard"
# Kabuğun yardımcı ikilileri (PATH dışı; $LIBEXEC, 0755): yalıtılmış görsel işçisi (framework
# ADR-0007). Framework'teki ayrı paket (JXL çözücüsü kabuk ikilisine girmez); kabuk onu çalışan
# ikiliye göreli ../lib/nuxut altında ve ~/.local/lib/nuxut altında arar.
HELPERS="nuxut-image-worker"
INSTALL_HELPERS=1

# Derleme için gerekenler (resmî depolar).
# cmake + qt6-base: Nuxut Qt stil eklentisi (ADR-0009; qt6-base Arch'ta başlıklarla gelir).
PKG_BUILD="base-devel git clang pkgconf wayland libxkbcommon mesa libglvnd fontconfig pipewire dbus cmake qt6-base"
# Görünüm için yazı tipleri (resmî depolar).
PKG_FONTS="ttf-jetbrains-mono-nerd inter-font adobe-source-serif-fonts"
# Birleşik görünümün varsayılan imleç ve ikon teması (resmî depolar). Yoksa Adwaita'ya düşülür.
# xdg-desktop-portal + gtk arka ucu: Nuxut portal arka ucunun ön ucu ve yedeği (nuxut-portals.conf).
# adw-gtk-theme: Nuxut GTK temasının tabanı (canlı GTK3 renkleri; docs/design/look-gtk.md).
PKG_LOOK="vimix-cursors papirus-icon-theme xdg-desktop-portal xdg-desktop-portal-gtk adw-gtk-theme"
# Material Symbols ikon fontu (resmî depo, 2026-10-16: AUR -git yerine; aynı aile "Material Symbols
# Rounded"). AUR -git sürümü kuruluysa dokunulmaz (genel kural: pkg_missing, -git/-bin varyantı).
PKG_ICONS="ttf-material-symbols-variable"
# İsteğe bağlı imleç seti (--with-cursors): Ayarlar'daki imleç seçiminde görünürler.
# Capitaine resmî depoda; diğerleri yalnız AUR'da: betik kurmaz, "AUR'da, kendiniz kurun" der.
PKG_CURSORS_REPO="capitaine-cursors"
PKG_CURSORS_AUR="material-cursors-git oreo-cursors-bin volantes-cursors qogir-cursor-theme sweet-cursors-git"

usage() {
    cat <<EOF
Kullanım: sh install.sh [seçenekler]

  (seçeneksiz)       kur ya da güncelle
  --no-deps          paket kurulumunu atla (bağımlılıklar zaten kurulu)
  --hyprland         (varsayılan) kabuk ilk açılışta hyprland.conf'a 'source' satırını ekler
  --no-hyprland      kabuk hyprland.conf'a dokunmasın (shell.toml [compositor] ensure_source = false);
                     --hyprland-takeover ile de ana ayar devralınmaz
  --hyprland-takeover    Hyprland ana ayarını Nuxut düzenine geçir (önce yedek; geri: --hypr-restore).
                     Kurulum/güncelleme bunu kendiliğinden yapmaz; ilk kurulumda Hoş geldin sorar
  --hypr-restore     Hyprland ana ayarının en yeni Nuxut yedeğini geri koy (o anki dosya da yedeklenir) ve çık
  -y, --yes          soruları evet kabul et
  --fix-path         gerekmez: kurulum dizinini oturumun PATH'ine Nuxut'un Hyprland katmanı
                     ekler. Yalnızca Hyprland dışı systemd/uwsm oturumları için ayrıca
                     ~/.config/environment.d/50-nuxut-path.conf yazar.
  --with-cursors     isteğe bağlı imleç temalarını da kur: Capitaine (depo); Material, Oreo,
                     Volantes, Qogir, Sweet (AUR: kurulmaz, komut önerilir). Varsayılan Vimix her zaman kurulur.
  --no-sharepicker   ~/.config/hypr/xdph.conf'a Nuxut ekran paylaşım seçicisini ekleme
                     (varsayılan: işaretli blokla eklenir, önce yedek; sizin seçiciniz varsa dokunulmaz)
  --no-filechooser   Nuxut dosya seçicisini varsayılan yapma; açık FileChooser seçiminiz (ör. kde) geçerli
                     kalır (varsayılan: Nuxut devralır, önceki seçim yedek; tercih sonraki kurulumlarda korunur)
  --keep-services    çakışan bar/bildirim servislerini (dunst, mako, swaync, waybar, quickshell,
                     ags, eww) devralma; yalnız uyar (varsayılan: önce kaydedilir, devre dışı
                     bırakılır, --uninstall geri getirir; docs/design/service-takeover.md)
  --no-restart       kurulumdan sonra çalışan kabuğu yeniden başlatma (varsayılan: çalışıyorsa
                     yeni sürümle yeniden başlatılır; çalışmıyorsa başlatılmaz)
  --reset-config     Nuxut'u ilk kez kurulmuş gibi başlat: ayarlar yedeğe taşınır
                     ($XDG_STATE_HOME/nuxut/backups/<zaman>/, son $BACKUP_KEEP yedek), görünüm
                     geri alınır, Hoş geldin yeniden açılır. Varsayılan: güncelleme ayarları
                     korur; eski biçimi kabuk açılışta taşır ve onarır (önce yedek;
                     geri al: nuxut-shell msg config undo)
  --keep-config      etkisiz (eski betiklerle uyum; güncelleme ayarları zaten korur)
  --dry-run          hiçbir şey kurmadan/yazmadan planı, PATH denetimini ve yeniden
                     başlatma planını göster
  --prefix DİZİN     kurulum öneki (varsayılan: ~/.local → ikililer ~/.local/bin)
  --src DİZİN        kaynak dizini (varsayılan: ~/.local/src/nuxut)
  --ref REF          kaynakları bir etikete ya da commit'e sabitle (dört depo; depo başına
                     NUXUT_REF_SHELL vb.). Varsayılan: dalın ucu; çekilen commit'ler yazdırılır
  --system-theme     önyükleme görünümünü (Plymouth, GRUB, Limine) Nuxut temasına geçir: planı
                     gösterir, onay ister; sistem dosyalarını (yardımcı, polkit eylemi, giriş
                     ekranı) kurar ve uygular (yönetici onayı Nuxut polkit penceresinde). Her
                     dosya yedeklenir; önyükleyici ayarı doğrulanmadan yazılmaz.
  --system-theme-revert
                     önyükleme temasını geri al (yedekler yerine konur, grub.cfg/initramfs yeniden üretilir)
  --uninstall        kurulan ikilileri, servis dosyalarını ve PATH düzeltmesini kaldır
                     (kaynaklar, ~/.config/nuxut ve yedekler kalır; taze kurulum çalışmaz)
  -h, --help         bu yardım

Ortam: NUXUT_GIT_BASE (varsayılan https://github.com/nuxut; ayna için), NUXUT_REPO_PREFIX (aynada
       depo adı öneki, ör. <önek>shell; varsayılan boş; ikisi de install.conf'ta hatırlanır),
       NUXUT_BRANCH, NUXUT_REF (ve NUXUT_REF_<DEPO>), NUXUT_CARGO
EOF
}

while [ $# -gt 0 ]; do
    case $1 in
        --no-deps) DEPS=0 ;;
        --hyprland) HYPR=yes ;;
        --no-hyprland) HYPR=no ;;
        -y|--yes) ASSUME_YES=1 ;;
        --fix-path) FIX_PATH=1 ;;
        --dry-run) DRY_RUN=1 ;;
        --no-restart) RESTART=0 ;;
        --keep-config) ;; # eski betiklerle uyum: güncelleme ayarları zaten korur
        --reset-config) RESET_CONFIG=1 ;;
        --with-cursors) CURSORS=1 ;;
        --no-sharepicker) SHAREPICKER=0 ;;
        --no-filechooser) FILECHOOSER=0 ;;
        --keep-services) TAKEOVER=0 ;;
        --prefix) [ $# -ge 2 ] || { echo "--prefix bir dizin ister." >&2; exit 2; }; PREFIX=$2; shift ;;
        --src) [ $# -ge 2 ] || { echo "--src bir dizin ister." >&2; exit 2; }; SRC=$2; shift ;;
        --ref) [ $# -ge 2 ] || { echo "--ref bir etiket ya da commit ister." >&2; exit 2; }; REF=$2; shift ;;
        --uninstall) ACTION=uninstall ;;
        --hyprland-takeover) HYPRLAND_TAKEOVER=1 ;;
        --hypr-restore) ACTION=hypr_restore ;;
        --system-theme) ACTION=system-theme ;;
        --system-theme-revert) ACTION=system-theme-revert ;;
        -h|--help) usage; exit 0 ;;
        *) echo "Bilinmeyen seçenek: $1" >&2; usage >&2; exit 2 ;;
    esac
    shift
done

# Kaynak ve önek: mutlak, sonra kanonik (sembolik bağ, `..`, `.`, sondaki `/` çözülür). Bütün
# yollar bu kanonik biçimden türer; güven denetimi ("Kaynak güveni") ve kullanım aynı gerçek
# dizindedir — denetimden sonra değiştirilen bir bağ sonucu etkilemez (güvenlik incelemesi
# path-trust-bypass, 2026-10-16).
case $SRC in
    /*) ;;
    *) echo "HATA: Kaynak dizini mutlak yol olmalı: $SRC (--src / NUXUT_SRC)" >&2; exit 1 ;;
esac
case $PREFIX in
    /*) ;;
    *) echo "HATA: Kurulum öneki mutlak yol olmalı: $PREFIX (--prefix / NUXUT_PREFIX)" >&2; exit 1 ;;
esac
SRC=$(realpath -m -- "$SRC") && PREFIX=$(realpath -m -- "$PREFIX") \
    || { echo "HATA: yol çözülemedi (realpath, coreutils)" >&2; exit 1; }
# Kabuk deposunun adı (yeniden exec denetimi): REPOS'tan, sabit değil (herkese açık ayna farklı
# adlandırabilir).
SHELL_REPO=""
for _pair in $REPOS; do [ "${_pair%%:*}" = shell ] && SHELL_REPO=${_pair#*:}; done

# --no-filechooser: türetilen portal seçiminde dosya seçici devralınmaz (ikili tercihi dosyaya yazar;
# bayraksız sonraki kurulum da kapalı tutar). Geri açmak: $PORTAL_BIN --write-portals-conf --filechooser.
PORTAL_FC_FLAG=
if [ "$FILECHOOSER" = 0 ]; then PORTAL_FC_FLAG=--no-filechooser; fi
BIN=$PREFIX/bin

# --hypr-restore: yalnızca yedeği geri koy (ADR-0007; kurulu kabukla, kaynak/derleme gerekmez).
if [ "$ACTION" = hypr_restore ]; then
    if [ ! -x "$BIN/nuxut-shell" ]; then echo "$BIN/nuxut-shell yok; önce kurun." >&2; exit 1; fi
    "$BIN/nuxut-shell" hypr restore-backup
    exit $?
fi
LIBEXEC=$PREFIX/lib/nuxut
DATA=${XDG_DATA_HOME:-$HOME/.local/share}
CONF=${XDG_CONFIG_HOME:-$HOME/.config}
# Portal arka ucu (docs/design/portal.md): ikili $LIBEXEC'te; .portal kaydı $XDG_DATA_HOME altında
# (xdg-desktop-portal 1.22 orayı da tarar; kabuk Nuxut oturumunda XDG_CURRENT_DESKTOP=Nuxut:Hyprland
# yalnızca bu dosya varsa koyar — src/services/portal_env.rs ile aynı yol).
PORTAL_BIN=xdg-desktop-portal-nuxut
PORTAL_FILE=$DATA/xdg-desktop-portal/portals/nuxut.portal
PORTAL_DBUS=$DATA/dbus-1/services/org.freedesktop.impl.portal.desktop.nuxut.service
PORTAL_UNIT=$CONF/systemd/user/xdg-desktop-portal-nuxut.service
PORTALS_CONF=$CONF/xdg-desktop-portal/nuxut-portals.conf
# Ekran paylaşım seçicisi (docs/design/screenshare-picker.md): ikili $LIBEXEC'te; xdph.conf bloğunu
# kaynaktaki betik yazar/kaldırır (yedekli, işaretli, idempotent).
PICKER_BIN=nuxut-sharepicker
# Dosya seçici (docs/design/file-chooser.md): portal FileChooser isteği başına başlatılan ikili.
CHOOSER_BIN=nuxut-filechooser
# Kök yetkili sistem yardımcısı (ADR-0008; açılış görünümü, docs/design/boot-theme.md): kökün
# dizinine (/usr/lib/nuxut) yalnız istenince kurulur: kurulum dosyaları $LIBEXEC/system'e hazırlar,
# Hoş geldin'in "Sistem görünümünü Nuxut'a geçir" adımı yönetici onayıyla kurar ve çalıştırır.
SYSHELPER_BIN=nuxut-system-helper
# Giriş ekranı (ADR-0010, docs/design/greeter.md): greetd'nin greeter'ı. Dosyaları kök
# yardımcıyla aynı yoldan /usr altına (greeter kullanıcısı ev dizinlerini okuyamaz); etkinleştirme Hoş geldin'de
# ya da --system-theme ile, yardımcının `greeter` alanı.
GREETER_BIN=nuxut-greeter
XDPH_PICKER_SH=$SRC/shell/dist/xdph/xdph-picker.sh
# Çakışan bar/bildirim servislerinin devralınması (docs/design/service-takeover.md).
TAKEOVER_SH=$SRC/shell/dist/takeover/conflicting-services.sh
# Ayarlar ikonu (settings dist/ altındaki ve $DATA altındaki göreli yol aynı).
SETTINGS_ICON=icons/hicolor/scalable/apps/nuxut-settings.svg

say()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
note() { printf '    %s\n' "$*"; }
warn() { printf '\033[1;33m!!\033[0m %s\n' "$*" >&2; }
die()  { printf '\033[1;31mHATA:\033[0m %s\n' "$*" >&2; exit 1; }

# >>> paket-seçimi (tests/fixtures/install_pkgs.sh bu bloğu sahte pacman/sudo ile ayrı çalıştırır)
# Eksik paketler `pacman -T` ile bulunur: provides'ı dikkate alır (ör. AUR'dan kurulu
# adw-gtk-theme-git `provides=adw-gtk-theme` ise karşılanmış sayılır). `pacman -S --needed <ad>`
# provides'a bakmaz; çakışan varyantı kaldırmayı sorar ve kurulumu durdururdu (2026-10-17).
# Provides etmeyen ama aynı adın -git/-bin varyantı kuruluysa paket atlanır (aynı dosyalar
# çakışırdı; ör. ttf-material-symbols-variable-git). Eksik yoksa pacman -S ve sudo hiç çağrılmaz.
# Sonuç PKG_NEED'de (boşlukla ayrılmış); atlananlar not olarak yazılır.
pkg_variant() { # $1 paket → kurulu -git/-bin varyantının adı (yoksa 1 döner)
    for s in -git -bin; do
        if pacman -Qq "$1$s" >/dev/null 2>&1; then printf '%s\n' "$1$s"; return 0; fi
    done
    return 1
}
pkg_missing() { # paketler → PKG_NEED
    PKG_NEED=""
    [ "$#" -gt 0 ] || return 0
    # pacman -T: hepsi karşılanmışsa 0, değilse 127 ve eksikleri satır satır yazar.
    miss=$(pacman -T "$@" 2>/dev/null) || true
    for p in $miss; do
        if v=$(pkg_variant "$p"); then
            note "$p: zaten $v kurulu, atlandı"
        else
            PKG_NEED="${PKG_NEED:+$PKG_NEED }$p"
        fi
    done
}
pkg_install() { # paketler → yalnız eksikleri kurar; hepsi varsa pacman -S/sudo çağrılmaz
    pkg_missing "$@"
    [ -n "$PKG_NEED" ] || { note "paketler tamam"; return 0; }
    note "kurulacak: $PKG_NEED"
    # shellcheck disable=SC2086
    sudo pacman -S --needed --noconfirm $PKG_NEED
}
PKG_FAILED=""
pkg_install_soft() { # pkg_install gibi; hata kurulumu durdurmaz, kurulamayanlar PKG_FAILED'e
    pkg_install "$@" && return 0
    # Tek çakışma bütün işlemi düşürür: kalanlar tek tek denenir, her hatanın nedeni yazılır.
    warn "Paketler birlikte kurulamadı; tek tek deneniyor."
    for p in $PKG_NEED; do
        if out=$(sudo pacman -S --needed --noconfirm "$p" 2>&1); then
            note "$p kuruldu"
        else
            why=$(printf '%s\n' "$out" | grep -v '^[[:space:]]*$' | tail -n 1)
            warn "$p kurulamadı, atlandı: ${why:-bilinmeyen hata}"
            PKG_FAILED="$PKG_FAILED $p"
        fi
    done
    return 0
}
# <<< paket-seçimi

ask() { # ask "soru" → 0 evet, 1 hayır
    [ "$ASSUME_YES" = 1 ] && return 0
    [ -t 0 ] || [ -r /dev/tty ] || return 1
    printf '%s [e/H] ' "$1"
    read -r ans < /dev/tty || return 1
    case $ans in e|E|evet|y|Y|yes) return 0 ;; *) return 1 ;; esac
}

# --- Kaynak güveni ------------------------------------------------------------------------
# Betik herkese açık `curl … | sh` giriş noktasıdır: derlenip çalıştırılacak kaynağın yeri ve
# kökeni doğrulanmadan kullanılmaz (güvenlik incelemesi 2026-10-16, docs/ARCHITECTURE.md
# "Kurulum"). Çalışma dizinine hiçbir yol göreli değildir; kaynak dizini ve üst dizinleri başka
# bir kullanıcının değiştirebileceği yerde olamaz; var olan klonların origin'i beklenen depo
# olmalıdır; uzak adres https:// ya da ssh:// (yerel ayna yalnız mutlak yol, açık NUXUT_GIT_BASE).

# Yol ve bütün üst dizinleri bize ya da root'a ait ve grup/başkaları yazamıyor. Yapışkan bitli
# paylaşımlı dizin (/tmp) de kabul edilmez (toctou-path-trust, 2026-10-16): kaynak ya da önek
# hiçbir zaman başkasının girdi oluşturabildiği bir dizinin altında olmaz; böylece denetimden
# sonra yolun hiçbir bileşenini bizden ve root'tan başkası değiştiremez. Olmayan kısım atlanır.
path_trusted() {
    p=$1
    while :; do
        if [ -e "$p" ]; then
            st=$(stat -L -c '%u %a' "$p" 2>/dev/null) || return 1
            u=${st%% *}; m=${st#* }
            [ "$u" = "$(id -u)" ] || [ "$u" = 0 ] || return 1
            case $m in *[2367]?|*[2367]) return 1 ;; esac
        fi
        [ "$p" = / ] && return 0
        p=$(dirname "$p")
    done
}

# Dizin bize ait, sembolik bağ değil ve grup/başkaları yazamıyor.
dir_private() {
    [ -d "$1" ] && [ ! -L "$1" ] || return 1
    st=$(stat -c '%u %a' "$1" 2>/dev/null) || return 1
    [ "${st%% *}" = "$(id -u)" ] || return 1
    case ${st#* } in *[2367]?|*[2367]) return 1 ;; esac
}

# scp biçimli adres (kullanıcı@makine:yol) ssh:// eşdeğerine çevrilir (forge'lardaki eşleme:
# kullanıcı@makine:sahip/depo.git = ssh://kullanıcı@makine/sahip/depo.git); diğerleri olduğu gibi.
url_canon() {
    case $1 in
        *://*|/*) printf '%s' "$1" ;;
        [!/]*@*:*) printf 'ssh://%s/%s' "${1%%:*}" "${1#*:}" ;;
        *) printf '%s' "$1" ;;
    esac
}

# Klonun origin'i beklenen depolardan biri mi ($1 dizin, $2 depo adı).
origin_trusted() {
    url=$(git -C "$1" config --get remote.origin.url 2>/dev/null) || return 1
    url=$(url_canon "$url")
    for base in "$GIT_BASE" "$GIT_HTTPS" "$GIT_SSH"; do
        [ "$url" = "$base/$2.git" ] && return 0
    done
    return 1
}

# Güncellenmiş install.sh'ye yeniden exec edilebilir mi: doğrulanmış klonda izlenen, değişmemiş,
# sembolik bağ olmayan dosya ve HEAD tam olarak seçilen commit (origin/$BRANCH ya da sabitleme).
reexec_target_trusted() {
    f=$SRC/shell/install.sh
    [ -f "$f" ] && [ ! -L "$f" ] || return 1
    origin_trusted "$SRC/shell" "$SHELL_REPO" || return 1
    git -C "$SRC/shell" ls-files --error-unmatch install.sh >/dev/null 2>&1 || return 1
    git -C "$SRC/shell" diff --quiet HEAD -- install.sh || return 1
    [ -n "$EXPECT_SHELL_HEAD" ] && [ "$(git -C "$SRC/shell" rev-parse HEAD)" = "$EXPECT_SHELL_HEAD" ]
}

# Dal adı seçenek gibi görünemez (`git fetch origin --upload-pack=…` komut çalıştırırdı).
case $BRANCH in
    ''|-*|*[!A-Za-z0-9._/-]*|*..*) die "Geçersiz dal adı: $BRANCH (NUXUT_BRANCH; [A-Za-z0-9._/-], '-' ile başlamaz)" ;;
esac
# Sabitleme referansı: dal adıyla aynı kural (seçenek gibi görünemez).
ref_valid() { case $1 in -*|*[!A-Za-z0-9._/-]*|*..*) return 1 ;; esac; }
ref_for() { # ref_for DİZİN → depo başına geçersiz kılma ya da NUXUT_REF/--ref
    case $1 in
        framework) printf '%s' "${NUXUT_REF_FRAMEWORK:-$REF}" ;;
        shell) printf '%s' "${NUXUT_REF_SHELL:-$REF}" ;;
        settings) printf '%s' "${NUXUT_REF_SETTINGS:-$REF}" ;;
        calendar) printf '%s' "${NUXUT_REF_CALENDAR:-$REF}" ;;
    esac
}
for _d in framework shell settings calendar; do
    ref_valid "$(ref_for "$_d")" || die "Geçersiz sabitleme ($_d): $(ref_for "$_d") (--ref / NUXUT_REF; [A-Za-z0-9._/-], '-' ile başlamaz)"
done
# git yalnız bu taşımaları kullanır: ext::/fd::/git:// ve http:// (HTTPS'ten düşürme dahil) yok.
# file: yalnız açıkça verilen mutlak yerel ayna (aşağıda) ve yerel klon.
export GIT_ALLOW_PROTOCOL=https:ssh:file
case $GIT_BASE in
    ''|https://?*|ssh://?*) ;;
    /*) [ -d "$GIT_BASE" ] || die "NUXUT_GIT_BASE yerel dizini yok: $GIT_BASE" ;;
    *) die "NUXUT_GIT_BASE yalnız https://, ssh:// ya da mutlak yerel yol olabilir: $GIT_BASE" ;;
esac
# İkililer önekin altına kurulur ve oturumda çalışır: önek de başkasının değiştiremeyeceği yerde.
path_trusted "$PREFIX" || die "Kurulum öneki ya da bir üst dizini başka bir kullanıcının değiştirebileceği yerde: $PREFIX (--prefix)."

# --- polkit ajanı -------------------------------------------------------------------------
# polkit oturum başına tek ajan kabul eder. Kullanıcının kendi ajanı (hyprpolkitagent…) varsa
# nuxut-polkit ETKİNLEŞTİRİLMEZ ve o ajana, servisine ya da Hyprland satırına dokunulmaz;
# geçiş kullanıcının elle yapacağı iştir (yönerge yazdırılır). Sonuç: bulunan polkit ajanının adı.
# soteria (GTK4), polkit-dumb-agent (uçbirim), cosmic-osd (COSMIC ajanı OSD sürecinde), budgie-polkit-dialog.
POLKIT_AGENTS="hyprpolkitagent polkit-gnome-authentication-agent-1 polkit-kde-authentication-agent-1 polkit-mate-authentication-agent-1 lxqt-policykit-agent lxpolkit xfce-polkit mate-polkit soteria polkit-dumb-agent cosmic-osd budgie-polkit-dialog"
other_polkit_agent() {
    for a in $POLKIT_AGENTS; do
        # comm 15 karakterle kesildiği için -x değil; yalnızca bu kullanıcının süreçleri.
        if pgrep -u "$(id -u)" -f "/$a( |\$)|^$a( |\$)" >/dev/null 2>&1; then echo "$a"; return 0; fi
    done
    if systemctl --user is-enabled --quiet hyprpolkitagent.service 2>/dev/null; then
        echo hyprpolkitagent.service; return 0
    fi
    # Hyprland zincirinde başlatan satır (Nuxut'un ürettiği dosyalar hariç).
    pat=$(printf '%s' "$POLKIT_AGENTS" | tr ' ' '|')
    if [ -d "$CONF/hypr" ] && grep -rIqsE "^[^#]*($pat)" "$CONF/hypr" \
            --exclude='nuxut.*' --exclude='*.kaldirildi' --exclude-dir=nuxut; then
        echo "Hyprland ayarı ($CONF/hypr)"; return 0
    fi
    return 1
}

# Portal seçim dosyası Nuxut'un mu (yok ya da ilk satırı portal/src/portals_conf.rs
# PORTALS_CONF_PREFIX ile başlıyor)? Önek sabit, ardındaki açıklama sürümle değişebilir.
# Kullanıcı ilk satırı silerek dosyayı sahiplenir; o zaman kurulum/kaldırma dokunmaz.
PORTALS_CONF_PREFIX="# nuxut-managed: portals"
portals_conf_ours() {
    [ ! -e "$PORTALS_CONF" ] && return 0
    case "$(head -n 1 "$PORTALS_CONF")" in "$PORTALS_CONF_PREFIX"*) return 0 ;; esac
    return 1
}

# --- PATH -------------------------------------------------------------------------------
# Kurulum dizini oturumun PATH'inde olmalı: terminalden 'nuxut-shell msg …' ve adla arayan
# betikler için (kabuk kendi ikililerini zaten mutlak yolla çağırır). Dizini Hyprland ortamına ve
# systemd kullanıcı ortamına Nuxut'un Hyprland katmanı koyar (bkz. path_check); environment.d
# dosyası yalnızca --fix-path ile, Hyprland dışı systemd/uwsm oturumları için yazılır.
ENVD_FILE=$CONF/environment.d/50-nuxut-path.conf

in_path() { # in_path DİZİN PATH-DEĞERİ
    case ":$2:" in *":$1:"*|*":$1/:"*) return 0 ;; *) return 1 ;; esac
}

# $BIN oturum PATH'ine (Hyprland keyword/eval, environment.d) girebilir mi? Güvenlik incelemesi
# 2026-10-17 (güvenilmeyen arama yolu): yalnız yalın mutlak yol (virgül, '$', ':', boşluk, tırnak,
# satır sonu yok; '.'/'..' bileşeni yok — keyword/eval enjeksiyonu olmaz), sistem dizini değil;
# dizin sembolik bağ değil, sahibi kullanıcı, grup/başkaları yazamaz; üst dizinleri kullanıcıya
# ya da root'a ait ve grup/başkaları yazamaz. Geçmezse PATH'e hiçbir şey eklenmez. Dizin her
# zaman PATH'in SONUNA eklenir: nuxut-* adları benzersiz, sistem komutları önce bulunur.
bin_path_safe() {
    case $BIN in /*) ;; *) return 1 ;; esac
    case $BIN in *[!A-Za-z0-9/._+-]*) return 1 ;; esac
    case /$BIN/ in */./*|*/../*) return 1 ;; esac
    case $BIN in /usr/bin|/bin|/usr/local/bin|/usr/sbin|/sbin) return 1 ;; esac
    [ "${#BIN}" -le 512 ] || return 1
    dir_private "$BIN" && path_trusted "$(dirname "$BIN")"
}

envd_line() { # environment.d satırı (dizin sonda); $BIN güvenli değilse boş (ve 1 döner)
    bin_path_safe || return 1
    # environment.d '$', '\', tırnak ve boşluğu özel yorumlar; bin_path_safe yalın yol garanti eder.
    # shellcheck disable=SC2016
    printf 'PATH=${PATH}:%s\n' "$BIN"
}

write_envd() {
    line=$(envd_line) || { warn "$BIN güvenilir değil (sahiplik, izin ya da özel karakter); PATH'e eklenmedi."; return 1; }
    if [ "$DRY_RUN" = 1 ]; then
        note "[deneme] yazılmadı: $ENVD_FILE ← $line"
        return 0
    fi
    mkdir -p "${ENVD_FILE%/*}"
    {
        printf '# Nuxut kurulum dizini (install.sh --fix-path). Kaldırmak için bu dosyayı silin.\n'
        printf '%s\n' "$line"
    } > "$ENVD_FILE.yeni"
    mv -f "$ENVD_FILE.yeni" "$ENVD_FILE"
    note "Yazıldı: $ENVD_FILE ($line)"
    note "Bir sonraki girişte etkin (systemd kullanıcı oturumu, uwsm, nuxut-shell.service)."
}

# Çalışan Hyprland oturumunun PATH'ine $BIN (2026-10-17). Hyprland env/envd'yi çalışırken de
# uygular (0.56.2 kaynağı: hyprlang handleEnv ve Lua hl.env ilk açılış dışında da setenv eder;
# envd/üçüncü argüman systemd + D-Bus etkinleştirme ortamına da aktarır). Lua ayarında `hyprctl
# eval` (Hyprland'in kendi PATH'i okunur, varsa eklenmez), hyprlang'da `hyprctl keyword envd`
# ($PATH Hyprland'in ortamından açılır). Yalnız Hyprland IPC; dosya yazılmaz. Hyprland'in
# başlattığı yeni süreçler hemen alır, açık terminaller almaz. Dizin PATH'in SONUNA ve yalnız
# bin_path_safe geçerse eklenir (güvenlik incelemesi 2026-10-17).
hypr_path_live() {
    [ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ] && command -v hyprctl >/dev/null 2>&1 || return 1
    # Lua dizgesine ve hyprlang değerine yalnız güvenilir, yalın yol girer.
    bin_path_safe || return 1
    lua="local p = os.getenv(\"PATH\") or \"\"; if not (\":\" .. p .. \":\"):find(\":$BIN:\", 1, true) then hl.env(\"PATH\", p .. \":$BIN\", true) end"
    out=$(hyprctl eval "$lua" 2>&1) || return 1
    case $out in
        *"only supported with the lua"*)
            # shellcheck disable=SC2016 # $PATH'i Hyprland açar
            out=$(hyprctl keyword envd "PATH,\$PATH:$BIN" 2>&1) || return 1 ;;
    esac
    [ "$out" = ok ]
}

path_check() {
    # Etkin PATH denetlenir (dosyanın varlığı değil): environment.d yalnızca systemd kullanıcı
    # yöneticisini besler; tty → bash → start-hyprland yolunda Hyprland'in ortamına girmez.
    # Kalıcı çözüm Nuxut'un Hyprland katmanında (nuxut.conf/nuxut.lua: oturumda bir kez
    # `envd = PATH,$PATH:<dizin>`, systemd'ye de aktarılır). Kullanıcı isteği 2026-10-16: kabuk
    # başlangıç dosyalarına (.bash_profile, .profile, fish) yazılmaz, elle adım istenmez.
    # NUXUT_SESSION_PATH testler için systemd ortamının yerine geçer.
    if [ -n "${NUXUT_SESSION_PATH+x}" ]; then
        session_path=$NUXUT_SESSION_PATH
    else
        session_path=$(systemctl --user show-environment 2>/dev/null | sed -n 's/^PATH=//p') || session_path=""
    fi
    case $BIN in
        /usr/bin|/bin|/usr/local/bin) return 0 ;;
    esac
    if [ -n "$session_path" ] && in_path "$BIN" "$session_path"; then
        note "$BIN oturumun PATH'inde."
        in_path "$BIN" "$PATH" || note "Bu terminal eski PATH'i taşıyor; yeni açılan terminallerde nuxut-shell çalışır."
        return 0
    fi
    if in_path "$BIN" "$PATH"; then
        note "$BIN bu terminalin PATH'inde."
        return 0
    fi
    if ! bin_path_safe; then
        warn "$BIN oturumun PATH'ine eklenmedi: güvenilir değil (sembolik bağ, başka sahip, grup/başkaları yazabiliyor ya da özel karakter)."
        return 0
    fi
    # Çalışan Hyprland oturumuna hemen eklenir (yeniden giriş beklenmez); kalıcı olanı Nuxut'un
    # Hyprland katmanı her girişte yapar. Kabuk başlangıç dosyalarına yazılmaz.
    if [ "$DRY_RUN" = 1 ]; then
        [ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ] && note "[deneme] $BIN çalışan Hyprland oturumunun PATH'ine eklenirdi (Hyprland IPC)."
    elif hypr_path_live; then
        note "$BIN çalışan Hyprland oturumunun PATH'ine eklendi: yeni açılan terminallerde nuxut-shell hemen çalışır"
        note "(açık terminaller eski PATH'i taşır). Kalıcı ekleme her girişte Nuxut'un Hyprland katmanından."
    else
        note "$BIN bu oturumun PATH'inde henüz yok. Yeni terminallerde etkin olması için oturumu yeniden açın;"
        note "Nuxut'un Hyprland katmanı onu girişte ekler. Sonra da yoksa: nuxut-shell msg hypr status"
    fi
    if [ "$FIX_PATH" = 1 ]; then
        # Açık istek: systemd kullanıcı oturumu/uwsm ile başlayan kurulumlar için ek yol.
        write_envd
    fi
    return 0
}

# --- Kabuğu yeniden başlatma ------------------------------------------------------------
# Yalnızca BAŞARILI kurulumdan sonra çağrılır (set -e: önceki her hata betiği orada bitirir).
# Hedef yalnızca varsayılan örnektir; NUXUT_SHELL_INSTANCE ayarlıysa o adlı örnek (IPC adı
# nuxut-shell-<ad>) ve o durumda systemd birimine hiç dokunulmaz (birim varsayılan örneğindir).
# Kabuk çalışmıyorsa hiçbir şey başlatılmaz. Ana repoya geçişte yeniden ele alınacak (t098).
INSTANCE=${NUXUT_SHELL_INSTANCE:-}
SHELL_UNIT=nuxut-shell.service
SHELL_LOG=${XDG_STATE_HOME:-$HOME/.local/state}/nuxut/nuxut-shell${INSTANCE:+-$INSTANCE}.log

shell_msg() { # yeni ikiliyle yalnızca hedef örneğe (NUXUT_SHELL_INSTANCE olduğu gibi geçer)
    "$BIN/nuxut-shell" msg "$@" </dev/null 2>/dev/null
}
shell_alive() { [ "$(shell_msg ping)" = pong ]; }
unit_active() {
    [ -z "$INSTANCE" ] && systemctl --user is-active --quiet "$SHELL_UNIT" 2>/dev/null
}
wait_for() { # wait_for KOŞUL ADIM (0,2 sn'lik adımlar) — koşul sağlanınca 0
    n=0
    while [ "$n" -lt "$2" ]; do
        $1 && return 0
        sleep 0.2
        n=$((n + 1))
    done
    $1
}
shell_gone() { ! shell_alive; }
start_hint() {
    if [ -n "$INSTANCE" ]; then
        note "Elle başlatmak için: NUXUT_SHELL_INSTANCE=$INSTANCE setsid -f $BIN/nuxut-shell"
    else
        note "Elle başlatmak için: systemctl --user start $SHELL_UNIT   (ya da: setsid -f $BIN/nuxut-shell)"
    fi
}

restart_plan() { # --dry-run: kabuğa hiçbir IPC gönderilmez, yalnızca plan
    if [ "$RESTART" = 0 ]; then
        note "atlanırdı (--no-restart)"
    elif [ "$ACTION" != install ]; then
        note "yok (kaldırma)"
    elif [ -z "${WAYLAND_DISPLAY:-}" ]; then
        note "atlanırdı: Wayland oturumu yok (TTY/ssh); kabuk bir sonraki girişte yeni sürümle başlar"
    elif [ -n "$INSTANCE" ]; then
        note "hedef: nuxut-shell-$INSTANCE örneği (NUXUT_SHELL_INSTANCE; systemd birimine dokunulmaz)"
        note "çalışıyorsa (msg ping): msg quit → kapanması beklenir → setsid ile ayrık başlatılır → msg ping ile doğrulanır"
        note "çalışmıyorsa: hiçbir şey başlatılmaz · günlük: $SHELL_LOG"
    elif unit_active; then
        note "hedef: $SHELL_UNIT (etkin) → systemctl --user restart $SHELL_UNIT → msg ping ile doğrulanır"
    else
        note "hedef: varsayılan örnek ($SHELL_UNIT etkin değil)"
        note "çalışıyorsa (msg ping): msg quit → kapanması beklenir → setsid ile ayrık başlatılır → msg ping ile doğrulanır"
        note "çalışmıyorsa: hiçbir şey başlatılmaz · günlük: $SHELL_LOG"
    fi
}

restart_shell() {
    # Taze kurulum kabuğu zaten kapattı: aynı yoldan başlat (Hoş geldin ilk açılış gibi gelir).
    case $FRESH_STOPPED in
        unit)
            if ! systemctl --user start "$SHELL_UNIT"; then
                warn "systemctl --user start $SHELL_UNIT başarısız (journalctl --user -u $SHELL_UNIT)."
                return 1
            fi
            if wait_for shell_alive 50; then
                note "Başlatıldı: $SHELL_UNIT"
                return 0
            fi
            warn "Kabuk başlatıldı ama 10 sn içinde yanıt vermedi (journalctl --user -u $SHELL_UNIT)."
            return 1 ;;
        proc) start_detached; return ;;
    esac
    if [ "$RESTART" = 0 ]; then
        note "Atlandı (--no-restart). Yeni sürüm kabuk yeniden başlayınca etkin olur."
        return 0
    fi
    if [ -z "${WAYLAND_DISPLAY:-}" ]; then
        note "Wayland oturumu yok (TTY/ssh): yeniden başlatma atlandı; kabuk bir sonraki girişte yeni sürümle başlar."
        return 0
    fi
    if unit_active; then
        if ! systemctl --user restart "$SHELL_UNIT"; then
            warn "systemctl --user restart $SHELL_UNIT başarısız (journalctl --user -u $SHELL_UNIT)."
            return 1
        fi
        if wait_for shell_alive 50; then
            note "Yeniden başlatıldı: $SHELL_UNIT"
            return 0
        fi
        warn "Kabuk yeniden başlatıldı ama 10 sn içinde yanıt vermedi (journalctl --user -u $SHELL_UNIT)."
        return 1
    fi
    if ! shell_alive; then
        note "Çalışan kabuk yok${INSTANCE:+ (örnek: $INSTANCE)}; hiçbir şey başlatılmadı."
        start_hint
        return 0
    fi
    if [ "$(shell_msg quit)" != ok ]; then
        warn "Çalışan kabuk 'msg quit' komutunu tanımıyor (eski sürüm); elle yeniden başlatın."
        note "Kapatın, sonra başlatın:"
        start_hint
        return 1
    fi
    if ! wait_for shell_gone 25; then
        warn "Eski kabuk 5 sn içinde kapanmadı; yeni sürüm başlatılmadı."
        return 1
    fi
    start_detached
}

start_detached() { # kabuğu oturumdan ayrık başlatır ve msg ping ile doğrular
    command -v setsid >/dev/null 2>&1 || { warn "setsid yok (util-linux); kabuk başlatılamadı."; start_hint; return 1; }
    mkdir -p "${SHELL_LOG%/*}"
    # Oturumdan ayrık: stdin /dev/null, çıktı günlüğe. Betiğin ortamından NUXUT_* (hedef örnek
    # adı hariç) ve derleme için verdiğimiz CARGO_TARGET_DIR aktarılmaz. LC_ALL=C: tr_TR
    # yereli sed aralığında [A-Z] "I"yı kapsamıyor (NUXUT_GIT_BASE kaçıyordu).
    set -- -u CARGO_TARGET_DIR
    for v in $(env | LC_ALL=C sed -n 's/^\(NUXUT_[A-Za-z0-9_]*\)=.*/\1/p'); do
        [ "$v" = NUXUT_SHELL_INSTANCE ] || set -- "$@" -u "$v"
    done
    if ! env "$@" setsid -f "$BIN/nuxut-shell" </dev/null >"$SHELL_LOG" 2>&1; then
        warn "Kabuk başlatılamadı."
        start_hint
        return 1
    fi
    if wait_for shell_alive 50; then
        note "Yeniden başlatıldı (yeni sürüm; günlük: $SHELL_LOG)"
        return 0
    fi
    warn "Yeni kabuk 10 sn içinde yanıt vermedi; şu an kabuk çalışmıyor olabilir. Günlük: $SHELL_LOG"
    start_hint
    return 1
}

# --- Taze kurulum (--reset-config) ----------------------------------------------------
# Yalnızca --reset-config ile (kurulum ya da güncelleme): (1) çalışan kabuk
# kendi yolundan kapanır (systemd birimi ya da yalnızca hedef örneğe msg quit), (2) YENİ kurulan
# ikiliyle 'nuxut-shell look revert' (ADR-0004 §8-3 kapatma yolu: GTK/Qt/imleç/dconf/kdeglobals/
# environment.d Nuxut öncesine; eski ikililerde bu komut yok, depo biçimi aynı nuxut-apply),
# (3) aşağıdaki AÇIK İZİN LİSTESİ yedeğe TAŞINIR, (4) kurulum sürer ve kabuk başlar: setup.toml
# yok → Hoş geldin ilk açılış gibi gelir.
#   $CONF/nuxut:  shell.toml, framework.toml (Ayarlar da yalnızca bu ikisini yazar). displays.toml
#                 KALIR: ekran yerleşimi donanıma ait (karar 2026-10-12)
#   $STATE/nuxut: setup.toml (ilk açılış), palette.toml (duvar kağıdı paleti), snapshots/
#                 (nuxut-apply deposu; YALNIZCA look revert tam başarılıysa — yoksa Nuxut öncesi
#                 değerleri taşıyan sabitlenmiş snapshot'lar yerinde kalır)
#   $XDG_RUNTIME_DIR/nuxut/welcome-<Hyprland imzası>: oturum içi "Hoş geldin açıldı" işareti (silinir)
# Joker yok. Aynı dizinlerdeki eski projenin dosyaları (shell.conf, shell/, quickshell/, *.conf,
# cosmetics/, tui.log …), takvim hesap/OAuth ve önbellek verisi ($XDG_DATA_HOME/nuxut), günlükler,
# apply.lock DOKUNULMAZ. Ayarlar'ın ve takvimin ayrı UI durum dosyası yok. Hyprland dahil etme
# satırı, nuxut.lua/conf ve hypr-source-added da kalır: kabuk açılışta dosyayı yeniden üretir;
# satırı silmek çalışan Hyprland'i kısayolsuz yeniden yükletirdi (Geri al: msg hypr undo).
STATE=${XDG_STATE_HOME:-$HOME/.local/state}
FRESH_CONF_ITEMS="shell.toml framework.toml"
FRESH_STATE_ITEMS="setup.toml palette.toml"
SNAPSHOTS=$STATE/nuxut/snapshots
BACKUPS=$STATE/nuxut/backups
FRESH_STOPPED=none   # taze kurulum kabuğu kapattıysa: unit | proc (sonda aynı yoldan başlatılır)
FRESH_DONE=0
FRESH_BACKUP=""

fresh_enabled() {
    [ "$ACTION" = install ] && [ "$RESET_CONFIG" = 1 ]
}

# Kurulum türü, ikililer kurulmadan ÖNCE: önceden kurulmuş Nuxut'un izi varsa güncelleme. Yalnızca
# çıktı içindir; Hoş geldin'i kabuk kendisi açar (setup.toml yoksa — yani gerçek ilk kurulumda ya
# da --reset-config sonrası). Kullanıcı isterse: nuxut-settings --welcome.
if [ -e "$STATE/nuxut/setup.toml" ] || [ -e "$CONF/nuxut/shell.toml" ] || [ -x "$BIN/nuxut-shell" ]; then
    INSTALL_KIND=update
else
    INSTALL_KIND=first
fi

exists() { [ -e "$1" ] || [ -L "$1" ]; } # kırık sembolik bağ da sayılır

welcome_marker() { # mutlak XDG_RUNTIME_DIR yoksa boş
    case ${XDG_RUNTIME_DIR:-} in
        /*) printf '%s/nuxut/welcome-%s\n' "$XDG_RUNTIME_DIR" "${HYPRLAND_INSTANCE_SIGNATURE:-}" ;;
    esac
}

is_backup_name() {
    case $1 in
        [0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]T[0-9][0-9][0-9][0-9][0-9][0-9]Z) return 0 ;;
        [0-9][0-9][0-9][0-9][0-9][0-9][0-9][0-9]T[0-9][0-9][0-9][0-9][0-9][0-9]Z-[0-9]*) return 0 ;;
        *) return 1 ;;
    esac
}

backups_newest_first() { # yalnızca bizim adlandırdığımız yedek dizinleri
    [ -d "$BACKUPS" ] || return 0
    for d in $(ls -1 "$BACKUPS" | LC_ALL=C sort -r); do
        is_backup_name "$d" && [ -d "$BACKUPS/$d" ] && [ ! -L "$BACKUPS/$d" ] && printf '%s\n' "$d"
    done
    return 0
}

fresh_plan() { # --dry-run: kabuğa IPC gönderilmez, hiçbir şey taşınmaz
    if ! fresh_enabled; then
        if [ "$INSTALL_KIND" = update ]; then
            note "güncelleme: ayarlar korunur (sıfırlamak için --reset-config); eski biçimi kabuk açılışta taşır ve onarır"
        else
            note "ilk kurulum: kabuk ilk açıldığında Hoş geldin gelir"
        fi
        return
    fi
    note "1) çalışan kabuk kapatılır (etkin $SHELL_UNIT: systemctl stop; değilse msg quit${INSTANCE:+, örnek: $INSTANCE});"
    note "   yeniden başlatılamayacaksa (--no-restart / Wayland yok / eski kabuk) taze kurulum atlanır"
    note "2) görünüm geri alınır: $BIN/nuxut-shell look revert (yeni sürüm)"
    note "3) taşınırdı → $BACKUPS/<zaman>/ (son $BACKUP_KEEP yedek tutulur):"
    any=0
    for f in $FRESH_CONF_ITEMS; do exists "$CONF/nuxut/$f" && { note "     $CONF/nuxut/$f"; any=1; }; done
    for f in $FRESH_STATE_ITEMS; do exists "$STATE/nuxut/$f" && { note "     $STATE/nuxut/$f"; any=1; }; done
    exists "$SNAPSHOTS" && { note "     $SNAPSHOTS (yalnızca look revert başarılıysa)"; any=1; }
    [ "$any" = 1 ] || note "     (taşınacak Nuxut dosyası yok)"
    n=0
    for d in $(backups_newest_first); do
        n=$((n + 1))
        # Bu çalıştırmanın yedeği de sayılır: eskilerden BACKUP_KEEP-1 kalır.
        [ "$any" = 1 ] && [ "$n" -ge "$BACKUP_KEEP" ] && note "   silinirdi (eski yedek): $BACKUPS/$d"
    done
    m=$(welcome_marker); [ -n "$m" ] && exists "$m" && note "   silinirdi (oturum işareti): $m"
    note "4) kurulum sürer, kabuk başlar → Hoş geldin ilk açılış gibi"
    note "dokunulmaz: eski proje dosyaları, $DATA/nuxut (takvim hesapları), Hyprland dahil etme satırı"
}

fresh_stop_shell() { # 0: kabuk kapalı; 1: kapatılamadı/kapatılmamalı → taze kurulum atlanır
    if unit_active; then
        if [ "$RESTART" = 0 ] || [ -z "${WAYLAND_DISPLAY:-}" ]; then
            warn "Kabuk ($SHELL_UNIT) çalışıyor ama yeniden başlatılamaz (--no-restart / Wayland oturumu yok): taze kurulum atlandı."
            return 1
        fi
        systemctl --user stop "$SHELL_UNIT" || { warn "systemctl --user stop $SHELL_UNIT başarısız: taze kurulum atlandı."; return 1; }
        FRESH_STOPPED=unit
        wait_for shell_gone 25 || { warn "Kabuk 5 sn içinde kapanmadı: taze kurulum atlandı."; return 1; }
        note "Kabuk kapatıldı ($SHELL_UNIT)."
        return 0
    fi
    if ! shell_alive; then
        note "Çalışan kabuk yok${INSTANCE:+ (örnek: $INSTANCE)}."
        return 0
    fi
    if [ "$RESTART" = 0 ] || [ -z "${WAYLAND_DISPLAY:-}" ]; then
        warn "Kabuk çalışıyor ama yeniden başlatılamaz (--no-restart / Wayland oturumu yok): taze kurulum atlandı."
        return 1
    fi
    if [ "$(shell_msg quit)" != ok ]; then
        warn "Çalışan kabuk 'msg quit' komutunu tanımıyor (eski sürüm): taze kurulum atlandı. Kabuğu kapatıp betiği yeniden çalıştırın."
        return 1
    fi
    if ! wait_for shell_gone 25; then
        warn "Kabuk 5 sn içinde kapanmadı: taze kurulum atlandı."
        return 1
    fi
    FRESH_STOPPED=proc
    note "Kabuk kapatıldı (msg quit${INSTANCE:+, örnek: $INSTANCE})."
}

fresh_revert_look() { # 0: geri alındı ya da gerek yoktu; 1: geri alınamadı
    if out=$("$BIN/nuxut-shell" look revert </dev/null 2>&1); then
        case $out in
            *'"report": null'*) note "Görünüm: Nuxut'un yazdığı bir şey yoktu." ;;
            *) note "Görünüm Nuxut öncesine döndü (GTK/Qt/imleç/dconf/kdeglobals/environment.d)." ;;
        esac
        return 0
    fi
    case $out in
        *'bilinmeyen argüman'*|*'kullanım: look apply'*)
            warn "Kurulu kabukta 'look revert' yok: görünüm geri alınmadı." ;;
        *)
            warn "Görünüm tam geri alınamadı (ayrıntı: $BIN/nuxut-shell look revert; tekrar çalıştırmak güvenli)."
    esac
    note "Snapshot deposu yerinde bırakıldı ($SNAPSHOTS): Nuxut öncesi değerler kaybolmaz; Ayarlar → Geri al ile dönülebilir."
    return 1
}

fresh_move() { # fresh_move KAYNAK HEDEF-DİZİNİ
    mkdir -p "$2"
    if mv -f "$1" "$2/"; then
        note "taşındı: $1"
    else
        warn "taşınamadı: $1 (yerinde kaldı)"
    fi
}

fresh_backup() { # fresh_backup REVERT_OK → FRESH_BACKUP (bir şey taşındıysa yedek dizini)
    any=0
    for f in $FRESH_CONF_ITEMS; do exists "$CONF/nuxut/$f" && any=1; done
    for f in $FRESH_STATE_ITEMS; do exists "$STATE/nuxut/$f" && any=1; done
    [ "$1" = 1 ] && exists "$SNAPSHOTS" && any=1
    [ "$any" = 1 ] || return 0
    mkdir -p "$BACKUPS"
    chmod 700 "$BACKUPS"
    stamp=$(date -u +%Y%m%dT%H%M%SZ)
    dir=$BACKUPS/$stamp
    n=1
    while ! mkdir -m 700 "$dir" 2>/dev/null; do
        [ "$n" -lt 100 ] || die "Yedek dizini oluşturulamadı: $BACKUPS"
        dir=$BACKUPS/$stamp-$n
        n=$((n + 1))
    done
    for f in $FRESH_CONF_ITEMS; do exists "$CONF/nuxut/$f" && fresh_move "$CONF/nuxut/$f" "$dir/config"; done
    for f in $FRESH_STATE_ITEMS; do exists "$STATE/nuxut/$f" && fresh_move "$STATE/nuxut/$f" "$dir/state"; done
    if [ "$1" = 1 ] && exists "$SNAPSHOTS"; then fresh_move "$SNAPSHOTS" "$dir/state"; fi
    {
        printf 'Nuxut taze kurulum (--reset-config) yedeği (install.sh, %s).\n' "$stamp"
        printf 'config/ → %s/nuxut/   state/ → %s/nuxut/\n' "$CONF" "$STATE"
        printf 'Geri yüklemek: kabuğu kapatın, dosyaları yerine taşıyın, kabuğu başlatın.\n'
    } > "$dir/BENIOKU.txt"
    FRESH_BACKUP=$dir
    # Son BACKUP_KEEP yedek kalır (yalnızca bizim adlandırdığımız dizinler).
    n=0
    for d in $(backups_newest_first); do
        n=$((n + 1))
        if [ "$n" -gt "$BACKUP_KEEP" ]; then
            rm -rf "${BACKUPS:?}/$d"
            note "eski yedek silindi: $BACKUPS/$d"
        fi
    done
}

fresh_reset() {
    fresh_stop_shell || return 0
    revert_ok=1
    fresh_revert_look || revert_ok=0
    fresh_backup "$revert_ok"
    m=$(welcome_marker)
    if [ -n "$m" ] && exists "$m"; then rm -f "$m"; fi
    FRESH_DONE=1
}

[ "$(id -u)" -ne 0 ] || die "root olarak çalıştırmayın; betik yalnız gerektiğinde yönetici parolası ister."

# --- Sistem görünümü (Plymouth / GRUB / Limine) ---------------------------------------
# docs/design/boot-theme.md, ADR-0008. Kaynak kurulumunun yolu; üründe paket kurar ve Hoş geldin'in
# "Sistem görünümünü Nuxut'a geçir" adımı uygular. Bu betik sisteme kendisi yazmaz ve sudo ile kök
# dosyası kurmaz: dosyaları hazırlar, kurulumu ve uygulamayı yönetici onayıyla nuxut-shell'e
# bırakır (önyükleyici yapılandırması yerine konmadan önce doğrulanır; her dosya yedeklenir).
SYSHELPER=/usr/lib/nuxut/nuxut-system-helper
# Kök dosyalarının hazırlığı (sudo yok; en az ayrıcalık kararı 2026-10-17): sistem yardımcısı,
# polkit eylemi ve giriş ekranı dosyaları kullanıcının dizinine, kurulacakları yolun aynısıyla
# ($SYSSTAGE/usr/…). Liste src/services/system_bundle.rs FILES ile aynı (dışındaki dosya kurulmaz).
# Kök dosya sistemine yalnız sistem görünümü ya da giriş ekranı istendiğinde, yönetici onayıyla
# (pkexec, Nuxut polkit penceresi) nuxut-shell kurar: Hoş geldin, Ayarlar ya da --system-theme.
SYSSTAGE=$LIBEXEC/system
system_stage() {
    G=$SRC/shell/dist/greeter
    S=$SRC/shell/dist/session
    T=$SYSSTAGE.yeni
    rm -rf "$T"
    install -d -m 755 "$T" \
        && install -Dm755 "$CARGO_TARGET_DIR/release/$SYSHELPER_BIN" "$T/usr/lib/nuxut/nuxut-system-helper" \
        && install -Dm644 "$SRC/shell/dist/polkit/org.nuxut.system-helper.policy" "$T/usr/share/polkit-1/actions/org.nuxut.system-helper.policy" \
        && install -Dm755 "$CARGO_TARGET_DIR/release/$GREETER_BIN" "$T/usr/bin/nuxut-greeter" \
        && install -Dm755 "$G/nuxut-greeter-session" "$T/usr/lib/nuxut/nuxut-greeter-session" \
        && install -Dm755 "$S/nuxut-session" "$T/usr/lib/nuxut/nuxut-session" \
        && install -Dm644 "$G/hyprland.lua" "$T/usr/share/nuxut/greeter/hyprland.lua" \
        && install -Dm644 "$G/tmpfiles.conf" "$T/usr/lib/tmpfiles.d/nuxut-greeter.conf" \
        && install -Dm644 "$S/nuxut.desktop" "$T/usr/share/wayland-sessions/nuxut.desktop" \
        && chmod -R go-w "$T" \
        && rm -rf "$SYSSTAGE" && mv "$T" "$SYSSTAGE"
}
system_theme_action() {
    shell_bin=$BIN/nuxut-shell
    [ -x "$shell_bin" ] || die "$shell_bin yok: önce kurulumu çalıştırın (sh install.sh)."
    if [ "$ACTION" = system-theme-revert ]; then
        say "Sistem görünümü: geri alma"
        [ -x "$SYSHELPER" ] || die "$SYSHELPER kurulu değil: geri alınacak bir şey yok."
        if [ "$DRY_RUN" = 1 ]; then
            "$shell_bin" system-theme revert --dry-run
            exit 0
        fi
        ask "Nuxut önyükleme teması ve giriş ekranı geri alınsın mı (yönetici parolası istenir)?" || die "vazgeçildi."
        rc=0
        "$shell_bin" system-theme revert || rc=$?
        # Giriş ekranı: kayıt varsa (Nuxut etkinleştirdiyse) önceki giriş yöntemi döner.
        if "$shell_bin" login-screen status --json 2>/dev/null | grep -q '"previous":"'; then
            "$shell_bin" login-screen revert || rc=$?
        fi
        exit "$rc"
    fi
    say "Sistem görünümü: plan (hiçbir şey değişmedi)"
    "$shell_bin" system-theme status
    note "Önizleme (sistem dosyasına dokunmaz): $shell_bin system-theme preview --out <dizin>"
    src_shell=$SRC/shell
    [ -f "$src_shell/Cargo.toml" ] || die "$src_shell yok: önce kurulumu çalıştırın."
    if [ "$DRY_RUN" = 1 ]; then
        note "[deneme] derlenirdi: $src_shell → nuxut-system-helper, nuxut-greeter (release); hazırlanırdı: $SYSSTAGE"
        "$shell_bin" system-theme install-helper --dry-run
        "$shell_bin" system-theme apply --dry-run
        "$shell_bin" login-screen apply --dry-run
        exit 0
    fi
    ask "Sistem dosyaları ($SYSHELPER, polkit eylemi, giriş ekranı) kurulsun, tema uygulansın mı (yönetici onayı istenir)?" || die "vazgeçildi."
    say "Sistem dosyaları derleniyor"
    # Kurulumla aynı ortak hedef dizini (derleme bölümündeki CARGO_TARGET_DIR).
    CARGO_TARGET_DIR="$SRC/target"; export CARGO_TARGET_DIR
    ( cd "$src_shell" && $CARGO build --locked --release -p nuxut-system-helper -p nuxut-greeter )
    system_stage || die "sistem dosyaları hazırlanamadı ($SYSSTAGE)"
    if command -v pacman >/dev/null 2>&1 && ! pacman -Qi greetd >/dev/null 2>&1; then
        sudo pacman -S --needed --noconfirm greetd || warn "greetd kurulamadı; giriş ekranı atlanır."
    fi
    # Sistem dosyaları eksik ya da eskiyse uygulama önce onları kurar (bir yönetici onayı daha).
    say "Tema uygulanıyor (yönetici onayı Nuxut polkit penceresinde)"
    rc=0
    "$shell_bin" system-theme apply || rc=$?
    say "Giriş ekranı uygulanıyor (yönetici onayıyla; önceki giriş yöntemi yedeklenir)"
    "$shell_bin" login-screen apply || rc=$?
    if [ "$rc" = 0 ]; then
        note "Görmek için yeniden başlatın. Geri almak için: sh install.sh --system-theme-revert"
    else
        warn "Uygulama tam bitmedi (çıkış $rc); başarısız hedefler kendi değişikliklerini geri aldı."
    fi
    exit "$rc"
}

case $ACTION in system-theme|system-theme-revert) system_theme_action ;; esac

# --- Deneme ----------------------------------------------------------------------------
if [ "$DRY_RUN" = 1 ]; then
    say "Deneme (--dry-run): hiçbir şey kurulmaz, indirilmez ya da yazılmaz"
    note "eylem: $ACTION · önek: $PREFIX · ikililer: $BIN ($BINS)"
    [ "$INSTALL_HELPERS" = 1 ] && note "yardımcılar: $LIBEXEC ($HELPERS, 0755)"
    note "kaynaklar: $SRC · dal: $BRANCH · paketler: $([ "$DEPS" = 1 ] && echo evet || echo hayır) · hyprland: $HYPR"
    if [ "$DEPS" = 1 ] && [ "$ACTION" = install ]; then
        note "paket listesi: $PKG_BUILD $PKG_FONTS $PKG_ICONS $PKG_LOOK"
        if command -v pacman >/dev/null 2>&1; then
            # pacman -T salt okunur; sudo çağrılmaz.
            # shellcheck disable=SC2086
            pkg_missing $PKG_BUILD $PKG_FONTS $PKG_ICONS $PKG_LOOK
            note "[deneme] kurulurdu: ${PKG_NEED:-hiçbiri (hepsi kurulu)}"
        fi
        [ "$CURSORS" = 1 ] && note "imleç seti: $PKG_CURSORS_REPO · AUR: $PKG_CURSORS_AUR"
    fi
    if [ "$ACTION" = install ] && [ "$HYPR" != no ] && [ "$HYPRLAND_TAKEOVER" = 1 ]; then
        note "[deneme] Hyprland ana ayarı devralınırdı: $BIN/nuxut-shell hypr takeover (yedek: hyprland.*.nuxut-yedek-TARİH; geri: --hypr-restore)"
    fi
    note "birimler: $CONF/systemd/user/{nuxut-shell,nuxut-polkit,nuxut-calendard}.service"
    other=$(other_polkit_agent) && note "polkit: başka ajan var ($other); nuxut-polkit etkinleştirilmezdi" || note "polkit: nuxut-polkit.service etkinleştirilirdi"
    note "uygulama girdileri: $DATA/applications/{nuxut-settings,nuxut-welcome}.desktop · ikon: $DATA/$SETTINGS_ICON"
    note "portal: $LIBEXEC/$PORTAL_BIN · $PORTAL_FILE · $PORTAL_DBUS · $PORTAL_UNIT · $PORTALS_CONF"
    note "ekran paylaşım seçicisi: $LIBEXEC/$PICKER_BIN"
    if [ "$SHAREPICKER" = 0 ] && [ "$ACTION" = install ]; then
        note "[deneme] xdph.conf'a dokunulmazdı (--no-sharepicker)"
    elif [ -f "$XDPH_PICKER_SH" ]; then
        if [ "$ACTION" = install ]; then
            sh "$XDPH_PICKER_SH" plan "$LIBEXEC/$PICKER_BIN" 2>&1 | while IFS= read -r l; do note "$l"; done || true
        else
            note "[deneme] xdph.conf'taki Nuxut bloğu (varsa) kaldırılırdı (önce yedek)"
        fi
    else
        note "[deneme] xdph.conf: kurulumda $CONF/hypr/xdph.conf'a seçici bloğu eklenir (yedekli; sizin seçiciniz varsa dokunulmaz)"
    fi
    if [ "$ACTION" = install ]; then
        # Kurulu ikili varsa üretilecek seçim dosyası (yalnızca stdout; hiçbir şey yazılmaz).
        # Yeni ikili henüz derlenmedi: eski sürüm bayrağı tanımıyorsa yalnızca not.
        if [ -x "$LIBEXEC/$PORTAL_BIN" ] && "$LIBEXEC/$PORTAL_BIN" --write-portals-conf --dry-run $PORTAL_FC_FLAG >/dev/null 2>&1; then
            note "[deneme] $PORTALS_CONF (kullanıcının Hyprland portal seçiminden türetilir):"
            "$LIBEXEC/$PORTAL_BIN" --write-portals-conf --dry-run $PORTAL_FC_FLAG 2>&1 | while IFS= read -r l; do note "    $l"; done
        else
            note "[deneme] $PORTALS_CONF kurulumda Hyprland portal seçiminden türetilir (--write-portals-conf)."
        fi
    elif ! portals_conf_ours; then
        note "[deneme] $PORTALS_CONF sizin (ilk satır işareti yok); kaldırmada kalırdı."
    fi
    if [ "$ACTION" = uninstall ]; then
        [ -f "$ENVD_FILE" ] && note "[deneme] silinirdi: $ENVD_FILE"
        [ -f "${XDG_STATE_HOME:-$HOME/.local/state}/nuxut/takeover/services" ] && note "[deneme] devralınan bar/bildirim servisleri geri getirilirdi"
        for h in $HELPERS; do [ -e "$LIBEXEC/$h" ] && note "[deneme] silinirdi: $LIBEXEC/$h"; done
        exit 0
    fi
    if [ "$TAKEOVER" = 1 ] && [ -f "$TAKEOVER_SH" ]; then
        say "Çakışan servisler (plan)"
        sh "$TAKEOVER_SH" plan 2>&1 | while IFS= read -r l; do note "$l"; done
    fi
    say "PATH"
    path_check || true
    say "Ayarlar (plan)"
    fresh_plan
    say "Kabuk yeniden başlatma (plan; kurulum başarılı olursa)"
    restart_plan
    exit 0
fi

# --- Kaldırma -------------------------------------------------------------------------
if [ "$ACTION" = uninstall ]; then
    say "Kaldırılıyor"
    for b in $BINS; do rm -f "$BIN/$b"; done
    for h in $HELPERS $PORTAL_BIN $PICKER_BIN $CHOOSER_BIN; do rm -f "$LIBEXEC/$h"; done
    # Hazırlanmış sistem dosyaları (kök dosya sistemindekiler --system-theme-revert ile kalır).
    rm -rf "$LIBEXEC/system"
    # Ekran paylaşım seçicisi: xdph.conf'taki yalnızca Nuxut bloğu (önce yedek).
    if [ -f "$XDPH_PICKER_SH" ]; then
        rc=0; out=$(sh "$XDPH_PICKER_SH" uninstall 2>&1) || rc=$?
        printf '%s\n' "$out" | while IFS= read -r l; do note "xdph: $l"; done
        [ "$rc" = 0 ] || warn "xdph.conf'taki Nuxut bloğu kaldırılamadı; elle silin (# >>> nuxut-sharepicker … # <<< nuxut-sharepicker)."
    else
        note "xdph.conf'ta Nuxut bloğu varsa elle silin (# >>> nuxut-sharepicker … # <<< nuxut-sharepicker)."
    fi
    rmdir "$LIBEXEC" 2>/dev/null || true
    systemctl --user disable --now nuxut-calendard.service nuxut-shell.service nuxut-polkit.service 2>/dev/null || true
    rm -f "$CONF/systemd/user/nuxut-calendard.service" "$CONF/systemd/user/nuxut-shell.service" \
          "$CONF/systemd/user/nuxut-polkit.service" \
          "$DATA/dbus-1/services/org.nuxut.Calendar1.service" \
          "$DATA/applications/nuxut-settings.desktop" "$DATA/applications/nuxut-welcome.desktop" \
          "$DATA/$SETTINGS_ICON" \
          "$DATA/applications/nuxut-calendar-webcal.desktop" \
          "$ENVD_FILE"
    # Portal: kayıt dosyaları; seçim dosyası yalnızca Nuxut'unkiyse (kullanıcı düzenlemediyse).
    rm -f "$PORTAL_FILE" "$PORTAL_DBUS" "$PORTAL_UNIT"
    rmdir "$(dirname "$PORTAL_FILE")" "$DATA/xdg-desktop-portal" 2>/dev/null || true
    if portals_conf_ours; then rm -f "$PORTALS_CONF"; fi
    systemctl --user daemon-reload 2>/dev/null || true
    for g in nuxut.conf nuxut.lua; do
        if [ -f "$CONF/hypr/$g" ]; then mv "$CONF/hypr/$g" "$CONF/hypr/$g.kaldirildi"; fi
    done
    note "Hyprland ayarındaki Nuxut satırını kaldırın (ya da önce: nuxut-shell msg hypr undo)."
    # Yeniden kurulumda kabuk source satırını yine ekleyebilsin.
    rm -f "${XDG_STATE_HOME:-$HOME/.local/state}/nuxut/hypr-source-added"
    # Devralınan bar/bildirim servisleri eski hâline (kayıttan).
    if [ -f "$TAKEOVER_SH" ]; then
        sh "$TAKEOVER_SH" restore 2>&1 | while IFS= read -r l; do note "geri getirme: $l"; done
    fi
    if [ -x "$BIN/nuxut-shell" ] && "$BIN/nuxut-shell" login-screen status --json 2>/dev/null | grep -q '"active":true'; then
        note "Giriş ekranı hâlâ Nuxut (greetd): geri almak için sh install.sh --system-theme-revert (önce, kaldırmadan)"
    fi
    note "Kaynaklar ($SRC) ve ayarlar (~/.config/nuxut) bırakıldı."
    exit 0
fi

# --- Ön kontroller ----------------------------------------------------------------------
IS_ARCH=0
command -v pacman >/dev/null 2>&1 && IS_ARCH=1
# Paketler kaynaklar çekilip betik kendini güncelledikten SONRA kurulur (aşağıda "Paketler"):
# eski bir kopya paket adımında takılıp yeni betiğe hiç ulaşamıyordu (2026-10-17). Kaynakları
# çekmek için yalnız git gerekir; yoksa önce yalnız o kurulur.
if ! command -v git >/dev/null 2>&1; then
    { [ "$DEPS" = 1 ] && [ "$IS_ARCH" = 1 ]; } || die "git yok."
    say "git (kaynakları çekmek için)"
    pkg_install git || die "git kurulamadı; önce git'i kurun."
fi

# --- Arch paketi (nuxut-git / nuxut) ------------------------------------------------------
# AUR paketi kullanıcının kendi seçimidir; betik AUR'dan hiçbir şey kurmaz ve AUR yardımcısı
# çağırmaz (güvenlik incelemesi 2026-10-16: doğrulanmamış AUR paketine güven). Paket kuruluysa
# yalnız `pacman -Qi` ile algılanır: güncelleme paket yöneticisinin işidir, betik kaynaktan
# kurmadan çıkar (iki kurulum /usr/bin ve ~/.local/bin yan yana kalmasın). Kaldırma ayrıdır.
if [ "$ACTION" = install ] && [ "$IS_ARCH" = 1 ]; then
    for p in nuxut-git nuxut; do
        if pacman -Qi "$p" >/dev/null 2>&1; then
            say "Nuxut '$p' paketiyle kurulu: güncellemeyi paket yöneticinle yap (ör. AUR yardımcın ya da makepkg)."
            note "Bu betik paket kuruluyken kaynaktan kurmaz. Kaynak kurulumu için önce '$p' paketini kaldırın."
            exit 0
        fi
    done
fi

# --- Kaynaklar --------------------------------------------------------------------------
say "Kaynaklar: $SRC"
path_trusted "$SRC" || die "Kaynak dizini ya da bir üst dizini başka bir kullanıcının değiştirebileceği yerde: $SRC (--src ile kendi dizininizi verin)."
mkdir -p "$SRC"
dir_private "$SRC" || die "$SRC size ait değil ya da başkaları yazabiliyor (chmod go-w ya da --src ile başka dizin)."
if [ "$MIRROR_FROM_CONF" = 1 ]; then
    say "kayıtlı kaynak: ${GIT_BASE:-https://github.com/nuxut} (önek: ${REPO_PREFIX:-yok}) — $INSTALL_CONF"
    note "Değiştirmek için NUXUT_GIT_BASE (ve NUXUT_REPO_PREFIX) verin; kayıt güncellenir."
fi
# Kayıtsız eski kurulum (2026-10-17): önceki sürümler aynayı install.conf'a yazmıyordu; GitHub'dan
# gelen betik var olan ayna klonlarını "beklenen depodan değil" diye reddediyordu. Ortam ve kayıt
# yokken bütün klonlar aynı tabanı (aynı önekle) gösteriyorsa ve taban https:// ya da ssh://
# ise terminalde sorulur: [E] bu kaynakla devam (başarılı çekişten sonra install.conf'a yazılır),
# [g] GitHub'a geç (yalnız temiz, yerel commit'siz klonlar; ileri sarılamazsa eski klon yedeğe
# taşınıp GitHub'dan yeniden indirilir), [h] dur. Terminal yoksa açık iletiyle durulur; karışık
# kaynaklar reddedilir. Sahiplik/izin denetimi git'ten önce, aynen.
GITHUB_SWITCH=0
if [ "$MIRROR_FROM_ENV" = 0 ] && [ "$MIRROR_FROM_CONF" = 0 ]; then
    found=0; mixed=0; old_base=""; old_prefix=""; old_list=""
    for dir in framework shell settings calendar; do
        [ -d "$SRC/$dir/.git" ] || continue
        { dir_private "$SRC/$dir" && dir_private "$SRC/$dir/.git"; } \
            || die "$SRC/$dir size ait değil ya da başkaları yazabiliyor; --src ile başka dizin verin."
        u=$(git -C "$SRC/$dir" config --get remote.origin.url 2>/dev/null) || u=""
        old_list="$old_list $dir=${u:-yok}"
        u=$(url_canon "$u"); n=${u##*/}; n=${n%.git}
        case $n in *"$dir") p=${n%"$dir"} ;; *) p="?" ;; esac
        if [ "$found" = 0 ]; then old_base=${u%/*}; old_prefix=$p; found=1
        elif [ "${u%/*}" != "$old_base" ] || [ "$p" != "$old_prefix" ]; then mixed=1
        fi
    done
    if [ "$found" = 1 ] && [ "$old_prefix" = "" ] \
        && { [ "$old_base" = "$GIT_HTTPS" ] || [ "$old_base" = "$GIT_SSH" ]; } && [ "$mixed" = 0 ]; then
        : # zaten herkese açık depolar
    elif [ "$found" = 1 ]; then
        [ "$mixed" = 0 ] && [ "$old_prefix" != "?" ] \
            || die "Mevcut klonlar farklı kaynakları gösteriyor:$old_list. Hepsini aynı kaynağa çevirin ya da --src ile başka dizin verin."
        case $old_base in
            *[[:space:][:cntrl:]]*) die "Mevcut kaynak adresinde boşluk ya da denetim karakteri var: $old_base" ;;
            https://?*|ssh://?*) ;;
            *) die "Mevcut kurulum desteklenmeyen bir kaynaktan ($old_base; yalnız https:// ya da ssh://). Kaynağı NUXUT_GIT_BASE ile verin ya da --src ile başka dizin seçin." ;;
        esac
        case $old_prefix in -*|*[!a-z0-9-]*) die "Mevcut klonların depo öneki geçersiz: $old_prefix" ;; esac
        if ! ( : < /dev/tty ) 2>/dev/null; then
            die "Mevcut kurulum $old_base kaynağından güncelleniyor ve bu kaynak kayıtlı değil. Onaylamak için betiği terminalde çalıştırın ya da kaynağı açıkça verin: NUXUT_GIT_BASE=$old_base NUXUT_REPO_PREFIX=$old_prefix"
        fi
        printf 'Mevcut kurulum %s kaynağından güncelleniyor. Bu kaynakla devam edilsin mi? [E]vet / [h]ayır / [g] GitHub'"'"'a geç: ' "$old_base"
        read -r ans < /dev/tty || ans=h
        case $ans in
            ''|e|E|evet|Evet|y|Y|yes)
                GIT_BASE=$old_base; REPO_PREFIX=$old_prefix
                # Yeniden exec edilen yeni betik de aynı kaynağı görsün; kayıt çekişten sonra.
                NUXUT_GIT_BASE=$GIT_BASE; NUXUT_REPO_PREFIX=$REPO_PREFIX
                export NUXUT_GIT_BASE NUXUT_REPO_PREFIX
                MIRROR_FROM_ENV=1 ;;
            g|G)
                # Önce hepsi denetlenir: biri uygun değilse hiçbir klona dokunulmaz.
                for dir in framework shell settings calendar; do
                    [ -d "$SRC/$dir/.git" ] || continue
                    git -C "$SRC/$dir" checkout --quiet -- Cargo.lock 2>/dev/null || true
                    [ -z "$(git -C "$SRC/$dir" status --porcelain)" ] \
                        || die "$SRC/$dir içinde yerel değişiklik var; GitHub'a geçilmedi (kaynak değiştirilmedi)."
                    [ -z "$(git -C "$SRC/$dir" rev-list -n 1 HEAD --not --remotes=origin 2>/dev/null)" ] \
                        || die "$SRC/$dir içinde eski kaynakta olmayan yerel commit var; GitHub'a geçilmedi (kaynak değiştirilmedi)."
                done
                GITHUB_SWITCH=1; REPO_PREFIX="" ;;
            *) die "Kurulum durduruldu; kaynak değiştirilmedi." ;;
        esac
        REPOS="framework:${REPO_PREFIX}framework shell:${REPO_PREFIX}shell settings:${REPO_PREFIX}settings calendar:${REPO_PREFIX}calendar"
        SHELL_REPO=${REPO_PREFIX}shell
    fi
fi
if [ -z "$GIT_BASE" ]; then
    # Herkese açık depolar (HTTPS); erişilemezse SSH.
    if GIT_TERMINAL_PROMPT=0 git ls-remote "$GIT_HTTPS/$SHELL_REPO.git" HEAD >/dev/null 2>&1; then
        GIT_BASE=$GIT_HTTPS
    else
        GIT_BASE=$GIT_SSH
        note "HTTPS erişimi yok; SSH kullanılıyor: $GIT_SSH"
    fi
fi
EXPECT_SHELL_HEAD=""
SOURCES_LINES=""
for pair in $REPOS; do
    dir=${pair%%:*}; repo=${pair#*:}
    if [ -d "$SRC/$dir/.git" ] && [ "$GITHUB_SWITCH" = 1 ]; then
        # GitHub'a geçiş (klonlar yukarıda temiz ve yerel commit'siz bulundu): ileri sarılabiliyorsa
        # ileri sarılır, değilse (ayrı geçmiş) eski klon yedeğe taşınıp yeniden indirilir.
        git -C "$SRC/$dir" remote set-url origin "$GIT_BASE/$repo.git"
        git -C "$SRC/$dir" fetch --quiet origin "$BRANCH" || die "$repo GitHub'dan çekilemedi ($GIT_BASE/$repo.git)."
        if ! git -C "$SRC/$dir" merge-base --is-ancestor HEAD "origin/$BRANCH" 2>/dev/null; then
            bak="$SRC/$dir.nuxut-yedek-$(date +%Y%m%d-%H%M%S)"
            mv "$SRC/$dir" "$bak"
            note "$dir: GitHub geçmişi farklı; eski klon yedekte: $bak"
        fi
    fi
    if [ -d "$SRC/$dir/.git" ]; then
        # Git komutlarından önce: klonun yapılandırması (kancalar, fsmonitor) komut çalıştırabilir.
        { dir_private "$SRC/$dir" && dir_private "$SRC/$dir/.git"; } \
            || die "$SRC/$dir size ait değil ya da başkaları yazabiliyor; --src ile başka dizin verin."
        origin_trusted "$SRC/$dir" "$repo" \
            || die "$SRC/$dir beklenen depodan değil (origin: $(git -C "$SRC/$dir" config --get remote.origin.url 2>/dev/null || echo yok); beklenen: $GIT_BASE/$repo.git). Doğru kaynağı NUXUT_GIT_BASE (ve NUXUT_REPO_PREFIX) ile verin ya da --src ile başka dizin seçin."
        # Derleme Cargo.lock'u yeniden yazmış olabilir; o bizim değişikliğimiz, geri al.
        git -C "$SRC/$dir" checkout --quiet -- Cargo.lock 2>/dev/null || true
        if [ -n "$(git -C "$SRC/$dir" status --porcelain)" ]; then
            die "$SRC/$dir içinde yerel değişiklik var; güncellemeden önce commit'leyin ya da kaldırın."
        fi
        git -C "$SRC/$dir" fetch --quiet origin "$BRANCH"
        git -C "$SRC/$dir" checkout --quiet "$BRANCH"
        git -C "$SRC/$dir" merge --quiet --ff-only "origin/$BRANCH" \
            || die "$dir: origin/$BRANCH ile hızlı ileri alınamadı (yerel commit var mı?)."
        note "$dir güncel: $(git -C "$SRC/$dir" log -1 --format='%h %s' | cut -c1-70)"
    else
        git clone --quiet --branch "$BRANCH" "$GIT_BASE/$repo.git" "$SRC/$dir" \
            || die "$repo indirilemedi ($GIT_BASE/$repo.git). Ağ bağlantısını denetleyin; ayna için NUXUT_GIT_BASE ve NUXUT_REPO_PREFIX verin."
        note "$dir indirildi."
    fi
    # Seçilen commit: sabitleme varsa etiket/commit (origin'den çekilen geçmişte), yoksa dal ucu.
    want=$(ref_for "$dir")
    if [ -n "$want" ]; then
        git -C "$SRC/$dir" fetch --quiet --tags origin "$BRANCH" || die "$dir: etiketler çekilemedi."
        c=$(git -C "$SRC/$dir" rev-parse --verify --quiet "$want^{commit}") \
            || die "$dir: '$want' bulunamadı (origin'de etiket ya da $BRANCH geçmişinde commit olmalı)."
        git -C "$SRC/$dir" checkout --quiet --detach "$c"
    else
        c=$(git -C "$SRC/$dir" rev-parse "origin/$BRANCH")
    fi
    [ "$(git -C "$SRC/$dir" rev-parse HEAD)" = "$c" ] || die "$dir: çalışma ağacı seçilen commit'te değil ($c)."
    [ "$dir" = shell ] && EXPECT_SHELL_HEAD=$c
    note "$dir: $c${want:+ (sabit: $want)}"
    SOURCES_LINES="$SOURCES_LINES$dir $repo $c ${want:--}
"
done

# Kaynak güncellemesi install.sh'yi değiştirdiyse yeni sürümle baştan (bir kez). Paket adımı
# bundan sonra olduğundan paketleri her zaman en yeni betik kurar (yeni sürüm yeni paket
# isteyebilir); taze kurulum/yedek derlemeden sonra olduğundan bir kez yapılır.
if [ -z "$REEXECED" ] && [ -n "$SELF_SUM" ] && [ -f "$SRC/shell/install.sh" ] \
    && [ "$(cksum < "$SRC/shell/install.sh")" != "$SELF_SUM" ]; then
    reexec_target_trusted || die "$SRC/shell/install.sh doğrulanamadı (origin/$BRANCH ile aynı değil ya da değiştirilmiş); yeniden exec edilmedi."
    say "install.sh güncellendi; yeni sürümle yeniden başlatılıyor"
    eval "set -- $ORIG_ARGS"
    export NUXUT_INSTALL_REEXEC=1
    exec sh "$SRC/shell/install.sh" "$@"
fi

# Kurulum kaydı: hangi depodan hangi commit (etiket gelene kadar tek doğrulanabilir kimlik).
# "durum kaynak" derleme başarılı olup ikililer doğrulanınca "durum kuruldu" olur.
SOURCES_FILE=$STATE/nuxut/install-sources
mkdir -p "$STATE/nuxut"
{
    printf '# Nuxut kurulum kaynakları (install.sh yazar)\n'
    printf 'zaman %s\ndal %s\nuzak %s\ndurum kaynak\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$BRANCH" "$GIT_BASE"
    printf '%s' "$SOURCES_LINES"
} > "$SOURCES_FILE.yeni" && mv -f "$SOURCES_FILE.yeni" "$SOURCES_FILE"
# Ortamla verilen ayna kaynaklar başarıyla çekildikten sonra hatırlanır (yukarıda "Ayna hatırlama").
if [ "$MIRROR_FROM_ENV" = 1 ]; then
    mkdir -p "$(dirname "$INSTALL_CONF")"
    rm -f "$INSTALL_CONF.yeni"
    ( umask 077; printf '# Nuxut kaynak aynası (install.sh yazar; NUXUT_GIT_BASE ile değişir)\nbase=%s\nprefix=%s\n' "$GIT_BASE" "$REPO_PREFIX" > "$INSTALL_CONF.yeni" )
    mv -f "$INSTALL_CONF.yeni" "$INSTALL_CONF"
    say "Kaynak aynası kaydedildi: $GIT_BASE (önek: ${REPO_PREFIX:-yok}) → $INSTALL_CONF"
    note "Sonraki çalıştırmalar ortam verilmezse bu kaynağı kullanır."
fi

# --- Paketler ---------------------------------------------------------------------------
if [ "$DEPS" = 1 ]; then
    if [ "$IS_ARCH" = 1 ]; then
        say "Paketler"
        # Rust: rustup varsa onu kullan, yoksa Arch'ın 'rust' paketi.
        RUST_PKG=""
        command -v rustup >/dev/null 2>&1 || command -v cargo >/dev/null 2>&1 || RUST_PKG=rust
        # Paket hatası kurulumu durdurmaz (2026-10-17): hangi paketin neden kurulamadığı yazılır,
        # kaynak/derleme sürer; eksik paket yüzünden derleme kırılırsa betik orada durur.
        # shellcheck disable=SC2086
        pkg_install_soft $PKG_BUILD $PKG_FONTS $PKG_ICONS $PKG_LOOK $RUST_PKG
        # AUR paketleri incelenmemiş PKGBUILD'lerdir (güvenlik incelemesi supply-chain, 2026-10-16):
        # betik AUR yardımcısı (paru/yay) ÇAĞIRMAZ. Kurulu olanlar yalnız `pacman -Qi` ile algılanır;
        # yalnız AUR'da olanlar için "kendiniz kurun" denir ve o özellik atlanır.
        if [ "$CURSORS" = 1 ]; then
            say "İmleç seti"
            # shellcheck disable=SC2086
            pkg_install_soft $PKG_CURSORS_REPO
            # shellcheck disable=SC2086
            pkg_missing $PKG_CURSORS_AUR
            missing_aur=$PKG_NEED
            [ -z "$missing_aur" ] || note "Şu imleç paketleri AUR'da, kendiniz kurun (PKGBUILD'i inceleyerek): $missing_aur — kurulmadıkça Ayarlar'da görünmezler."
        fi
    else
        warn "Arch dışı sistem: paketleri kendiniz kurun (Arch adları): $PKG_BUILD $PKG_FONTS $PKG_ICONS $PKG_LOOK rust"
    fi
fi

# Rust araç zinciri yalnız pacman paketleriyle: `rust` ya da `rustup` (pacman). Betik rustup-init'i
# `curl | sh` ile indirmez; rustup kuruluysa araç zincirini rustup kendisi (sağlama toplamıyla) çeker.
if command -v rustup >/dev/null 2>&1; then
    rustup toolchain list | grep -q . || { say "Rust araç zinciri (rustup default stable)"; rustup default stable; }
fi
command -v cargo >/dev/null 2>&1 || die "cargo bulunamadı (rust ya da rustup kurun)."
RUSTV=$(rustc --version | awk '{print $2}')
case $RUSTV in
    1.[0-7][0-9].*|1.8[0-4].*) die "Rust $RUSTV çok eski; en az 1.85 (edition 2024) gerekir: rustup update ya da pacman -Syu rust" ;;
esac

# --- Derleme ----------------------------------------------------------------------------
[ -z "$PKG_FAILED" ] || warn "Kurulamayan paketler:$PKG_FAILED — derleme bunlar olmadan kırılırsa önce bunları kurun, sonra betiği yeniden çalıştırın."
# --locked (güvenlik incelemesi supply-chain, 2026-10-16): yalnız depodaki Cargo.lock'un sürüm ve
# sağlama toplamlarıyla derlenir; kilit eskiyse derleme durur, kendiliğinden yeni sürüm çekilmez.
# Üç çalışma alanı da framework'ü yoldan kullanır; ortak hedef dizini onu tek kez derler.
export CARGO_TARGET_DIR="$SRC/target"
# `nuxut-shell doctor` derlenen commit'i gösterir (option_env!; değişince cargo yeniden derler).
NUXUT_BUILD_COMMIT=$(git -C "$SRC/shell" rev-parse --short=12 HEAD 2>/dev/null || true)
export NUXUT_BUILD_COMMIT
say "Derleniyor (ilk seferde birkaç dakika sürer)"
# polkit ajanı ve kilit ekranı aynı çalışma alanında ayrı paketler; sahte test ikilileri özellik
# ister (fake-auth), derlenmez.
( cd "$SRC/shell"    && $CARGO build --locked --release -p nuxut-shell -p nuxut-lock -p nuxut-polkit -p nuxut-system-helper -p nuxut-greeter )
# Portal arka ucu: kabuk çalışma alanının ayrı üyesi (UI'sız küçük ikili).
( cd "$SRC/shell"    && $CARGO build --locked --release -p "$PORTAL_BIN" )
# Görsel işçisi framework'teki ayrı paket (kabuk [[bin]] tanımlamaz); aynı framework kaynağından
# derlenir (el sıkışma sürüm eşliği ister), ortak hedef dizinine.
[ "$INSTALL_HELPERS" = 1 ] && $CARGO build --locked --release --manifest-path "$SRC/framework/Cargo.toml" -p nuxut-image-worker
( cd "$SRC/settings" && $CARGO build --locked --release )
( cd "$SRC/calendar" && $CARGO build --locked --release -p nuxut-calendard )

# Qt stili (ADR-0009, qt-style/): Qt6 için (Qt5 kuruluysa onun için de) aynı kaynaktan C++ eklentisi.
# cmake + qt6-base PKG_BUILD'de (resmî depo); Qt5 eklentisi yalnızca qt5-base zaten kuruluysa
# derlenir (Qt5 uygulaması yoksa Qt5 kurulmaz). Derleme başarısızsa kurulum durmaz: uyarı + günlük
# yolu, Qt uygulamaları Breeze/Fusion ile sürer. Kurulum $LIBEXEC/qt<N>/styles/; kabuk eklentiyi
# orada bulunca kdeglobals widgetStyle=Nuxut ve QT_PLUGIN_PATH yazar. Sonda doğrulanır.
QT_STYLE_BUILT=""
QT_STYLE_FAILED=""
if command -v cmake >/dev/null 2>&1; then
    mkdir -p "$CARGO_TARGET_DIR"
    for qv in 6 5; do
        if [ ! -d "/usr/lib/cmake/Qt${qv}Widgets" ]; then
            [ "$qv" = 6 ] && { warn "Qt6 geliştirme dosyaları yok (qt6-base paketi): Nuxut Qt stili derlenmedi; Qt uygulamaları Breeze/Fusion ile sürer."; QT_STYLE_FAILED="$QT_STYLE_FAILED 6"; }
            continue
        fi
        bdir="$CARGO_TARGET_DIR/qt-style-qt$qv"
        qlog="$CARGO_TARGET_DIR/qt-style-qt$qv.log"
        if cmake -S "$SRC/shell/qt-style" -B "$bdir" -DQT_MAJOR=$qv -DBUILD_TESTING=OFF -DCMAKE_BUILD_TYPE=Release > "$qlog" 2>&1 \
            && cmake --build "$bdir" -j "$(nproc 2>/dev/null || echo 2)" >> "$qlog" 2>&1 \
            && [ -f "$bdir/plugins/styles/libnuxut-style.so" ]; then
            QT_STYLE_BUILT="$QT_STYLE_BUILT $qv"
        else
            QT_STYLE_FAILED="$QT_STYLE_FAILED $qv"
            warn "Nuxut Qt$qv stili derlenemedi (günlük: $qlog); Qt$qv uygulamaları Breeze/Fusion ile sürer."
            tail -n 5 "$qlog" 2>/dev/null | while IFS= read -r l; do note "qt$qv: $l"; done
        fi
    done
else
    QT_STYLE_FAILED=" 6"
    warn "cmake yok (cmake paketi): Nuxut Qt stili derlenmedi; Qt uygulamaları Breeze/Fusion ile sürer."
fi

# --- Kurulum ----------------------------------------------------------------------------
say "Kuruluyor: $BIN"
mkdir -p "$BIN"
for b in $BINS; do
    # Önce geçici ada kopyala, sonra yer değiştir: çalışan örnek etkilenmez.
    install -m 755 "$CARGO_TARGET_DIR/release/$b" "$BIN/.$b.yeni"
    mv -f "$BIN/.$b.yeni" "$BIN/$b"
done
if [ "$INSTALL_HELPERS" = 1 ]; then
mkdir -p "$LIBEXEC"
for h in $HELPERS; do
    install -m 755 "$CARGO_TARGET_DIR/release/$h" "$LIBEXEC/.$h.yeni"
    mv -f "$LIBEXEC/.$h.yeni" "$LIBEXEC/$h"
done
note "Yardımcılar: $LIBEXEC ($HELPERS)"
fi
for qv in $QT_STYLE_BUILT; do
    mkdir -p "$LIBEXEC/qt$qv/styles"
    install -m 755 "$CARGO_TARGET_DIR/qt-style-qt$qv/plugins/styles/libnuxut-style.so" "$LIBEXEC/qt$qv/styles/.libnuxut-style.so.yeni"
    mv -f "$LIBEXEC/qt$qv/styles/.libnuxut-style.so.yeni" "$LIBEXEC/qt$qv/styles/libnuxut-style.so"
    note "Qt$qv stili: $LIBEXEC/qt$qv/styles/libnuxut-style.so (yeni açılan Qt uygulamalarında)"
done
# Sistem dosyaları (yardımcı, polkit eylemi, giriş ekranı): yalnız hazırlanır, kök dosya sistemine
# dokunulmaz ve sudo istenmez (en az ayrıcalık kararı 2026-10-17). Kurulum, sistem görünümü ya da
# giriş ekranı istendiğinde yönetici onayıyla (Hoş geldin, Ayarlar, --system-theme).
if system_stage; then
    note "Sistem dosyaları hazırlandı: $SYSSTAGE (yönetici onayıyla yalnız istenince kurulur)"
    if "$BIN/nuxut-shell" system-theme status --json 2>/dev/null | grep -q '"helper_update": true'; then
        note "Kurulu sistem dosyalarının yeni sürümü hazır: Ayarlar › Sistem görünümü'nden güncellenir."
    fi
else
    warn "Sistem dosyaları hazırlanamadı; açılış görünümü ve giriş ekranı adımları kullanılamaz."
fi

# Taze kurulum (--reset-config): yeni ikililer yerinde, servis/Hyprland adımlarından önce (--no-hyprland
# shell.toml'u taze dizine yazsın). Derleme başarısızsa buraya hiç gelinmez: ayarlar kalır.
if fresh_enabled; then
    say "Taze kurulum: ayarlar yedeğe taşınıyor (--reset-config)"
    fresh_reset
fi

# Takvim servisi: D-Bus etkinleştirmesi + systemd kullanıcı birimi (ilk istekte başlar).
mkdir -p "$CONF/systemd/user" "$DATA/dbus-1/services" "$DATA/applications"
sed "s#^ExecStart=.*#ExecStart=$BIN/nuxut-calendard#" \
    "$SRC/calendar/dist/nuxut-calendard.service" > "$CONF/systemd/user/nuxut-calendard.service"
sed "s#^Exec=.*#Exec=$BIN/nuxut-calendard#" \
    "$SRC/calendar/dist/org.nuxut.Calendar1.service" > "$DATA/dbus-1/services/org.nuxut.Calendar1.service"
if [ -f "$SRC/calendar/dist/nuxut-calendar-webcal.desktop" ]; then
    sed "s#/usr/bin/#$BIN/#g" "$SRC/calendar/dist/nuxut-calendar-webcal.desktop" \
        > "$DATA/applications/nuxut-calendar-webcal.desktop"
fi
# Kabuk: systemd kullanıcı birimi (ADR-0003). Oturum başında Hyprland'deki nuxut.lua/nuxut.conf
# kancası ortamı aktarıp başlatır; graphical-session.target'ı başlatan oturum yöneticisi (uwsm)
# varsa birim etkinleştirilir, yoksa kanca yeter.
sed "s#^ExecStart=.*#ExecStart=$BIN/nuxut-shell#" \
    "$SRC/shell/dist/nuxut-shell.service" > "$CONF/systemd/user/nuxut-shell.service"
systemctl --user daemon-reload 2>/dev/null || true
if command -v uwsm >/dev/null 2>&1; then
    if systemctl --user enable nuxut-shell.service 2>/dev/null; then note "nuxut-shell.service etkinleştirildi (uwsm oturumu)."; fi
fi

# polkit ajanı: birim her zaman kurulur; ETKİNLİK kullanıcının seçimidir. Hyprland kancası
# (nuxut.lua/nuxut.conf) birimi yalnızca etkinse başlatır. İlk kurulumda başka polkit ajanı yoksa
# etkinleştirilir; zaten etkinse (güncelleme) korunur ve çalışıyorsa yeni ikiliyle yeniden başlar.
sed "s#^ExecStart=.*#ExecStart=$BIN/nuxut-polkit#" \
    "$SRC/shell/polkit/dist/nuxut-polkit.service" > "$CONF/systemd/user/nuxut-polkit.service"
systemctl --user daemon-reload 2>/dev/null || true
if systemctl --user is-enabled --quiet nuxut-polkit.service 2>/dev/null; then
    systemctl --user try-restart nuxut-polkit.service 2>/dev/null || true
    if other=$(other_polkit_agent); then
        warn "polkit: nuxut-polkit etkin, ama başka ajan da var ($other) — oturum başında önce kaydolan kazanır. Birini kapatın."
    fi
elif other=$(other_polkit_agent); then
    note "polkit: başka bir ajan kullanılıyor ($other); nuxut-polkit etkinleştirilmedi, ona dokunulmadı."
    note "  Nuxut parola diyaloğuna geçmek için: o ajanı Hyprland exec-once satırınızdan/servisinden kaldırın,"
    note "  sonra: systemctl --user enable --now nuxut-polkit.service   (geri dönüş: disable --now)"
elif systemctl --user enable nuxut-polkit.service 2>/dev/null; then
    note "polkit: nuxut-polkit.service etkinleştirildi (sonraki girişte başlar; şimdi: systemctl --user start nuxut-polkit.service)."
fi

# Portal arka ucu (docs/design/portal.md): ikili, .portal kaydı (ayrı kök), D-Bus etkinleştirmesi,
# systemd birimi (etkinleştirilmez; D-Bus başlatır) ve Nuxut oturumunun portal seçimi. Kullanıcının
# hyprland-portals.conf'una dokunulmaz. xdg-desktop-portal ortamı ve portal listesini açılışta
# okur: etkisi bir sonraki girişte (çalışan portal yeniden başlatılmaz).
mkdir -p "$LIBEXEC" "$(dirname "$PORTAL_FILE")" "$CONF/xdg-desktop-portal"
install -m 755 "$CARGO_TARGET_DIR/release/$PORTAL_BIN" "$LIBEXEC/.$PORTAL_BIN.yeni"
mv -f "$LIBEXEC/.$PORTAL_BIN.yeni" "$LIBEXEC/$PORTAL_BIN"
install -m 644 "$SRC/shell/dist/portal/nuxut.portal" "$PORTAL_FILE"
sed "s#^Exec=.*#Exec=$LIBEXEC/$PORTAL_BIN#" \
    "$SRC/shell/dist/portal/org.freedesktop.impl.portal.desktop.nuxut.service" > "$PORTAL_DBUS"
sed "s#^ExecStart=.*#ExecStart=$LIBEXEC/$PORTAL_BIN#" \
    "$SRC/shell/dist/portal/xdg-desktop-portal-nuxut.service" > "$PORTAL_UNIT"
systemctl --user daemon-reload 2>/dev/null || true
# Seçim dosyası sabit değil: ikili, Nuxut olmasaydı Hyprland için seçilecek dosyadan
# (hyprland-portals.conf/portals.conf, xdg-desktop-portal arama sırasıyla) türetir — kullanıcının
# seçimleri (ör. FileChooser=kde) korunur, yalnızca Settings başına nuxut eklenir. Yalnızca yoksa
# ya da ilk satır işareti duruyorsa yazar (atomik); kullanıcının dosyalarını yalnızca okur.
# Dosya seçici ikilisi portalın yanında (portal onu istek başına başlatır).
install -m 755 "$CARGO_TARGET_DIR/release/$CHOOSER_BIN" "$LIBEXEC/.$CHOOSER_BIN.yeni"
mv -f "$LIBEXEC/.$CHOOSER_BIN.yeni" "$LIBEXEC/$CHOOSER_BIN"
# Nuxut'un önceki seçim dosyası değişecekse yedek (kullanıcı dosyalarına zaten yazılmaz).
if [ -f "$PORTALS_CONF" ] && portals_conf_ours \
    && new=$("$LIBEXEC/$PORTAL_BIN" --write-portals-conf --dry-run $PORTAL_FC_FLAG 2>/dev/null) \
    && [ "$new" != "$(cat "$PORTALS_CONF")" ]; then
    cp -p "$PORTALS_CONF" "$PORTALS_CONF.nuxut-yedek-$(date +%Y%m%d%H%M%S)"
fi
if out=$("$LIBEXEC/$PORTAL_BIN" --write-portals-conf $PORTAL_FC_FLAG 2>&1); then
    note "$out"
else
    warn "$PORTALS_CONF üretilemedi ($out); Nuxut oturumunda görünüm portaldan gitmeyebilir."
fi
note "Portal arka ucu kuruldu: görünüm (koyu/açık, vurgu) bir sonraki girişte uygulamalara canlı gider."

# Ekran paylaşım seçicisi (docs/design/screenshare-picker.md): ikili $LIBEXEC'te (nuxut-shell paketinin
# ikinci ikilisi, yukarıda derlendi). xdph onu ~/.config/hypr/xdph.conf'taki
# screencopy:custom_picker_binary ile çağırır; blok işaretli ve yedekli yazılır, kullanıcının kendi
# seçicisi varsa ya da dosya sembolik bağsa dokunulmaz. xdph ayarı açılışta okur: etkisi sonraki girişte.
install -m 755 "$CARGO_TARGET_DIR/release/$PICKER_BIN" "$LIBEXEC/.$PICKER_BIN.yeni"
mv -f "$LIBEXEC/.$PICKER_BIN.yeni" "$LIBEXEC/$PICKER_BIN"
if [ "$SHAREPICKER" = 1 ]; then
    rc=0; out=$(sh "$XDPH_PICKER_SH" install "$LIBEXEC/$PICKER_BIN" 2>&1) || rc=$?
    printf '%s\n' "$out" | while IFS= read -r l; do note "xdph: $l"; done
    case $rc in
        0) note "Ekran paylaşım seçicisi: Nuxut (sonraki girişte; şimdi: systemctl --user restart xdg-desktop-portal-hyprland)." ;;
        3) note "Ekran paylaşım seçicisi: xdph.conf'a dokunulmadı (yukarıdaki nedene bakın)." ;;
        *) warn "xdph.conf güncellenemedi; ekran paylaşımı xdph'in kendi seçicisiyle sürer." ;;
    esac
else
    note "Ekran paylaşım seçicisi: --no-sharepicker; xdph.conf'a dokunulmadı (ikili: $LIBEXEC/$PICKER_BIN)."
fi

# Zorunlu ikililerin hepsi yerinde mi (2026-10-16: nuxut-lock listede yoktu, kurulum "Tamam" dedi
# ama Kilitle hiçbir şey açmıyordu). Eksik varsa kurulum hatayla biter. Liste ile çalışma alanının
# [[bin]]'leri tests/it/install_bins.rs'te eşlenir.
missing=""
for b in $BINS; do [ -x "$BIN/$b" ] || missing="$missing $BIN/$b"; done
for h in $PORTAL_BIN $PICKER_BIN $CHOOSER_BIN; do [ -x "$LIBEXEC/$h" ] || missing="$missing $LIBEXEC/$h"; done
if [ "$INSTALL_HELPERS" = 1 ]; then
    for h in $HELPERS; do [ -x "$LIBEXEC/$h" ] || missing="$missing $LIBEXEC/$h"; done
fi
[ -z "$missing" ] || die "Kurulum eksik kaldı, şu ikililer yok:$missing"
note "İkililer doğrulandı: $BINS · $PORTAL_BIN $PICKER_BIN $CHOOSER_BIN"
sed -i 's/^durum kaynak$/durum kuruldu/' "$SOURCES_FILE" 2>/dev/null || true
# Nuxut Qt stili (ADR-0009): isteğe bağlı, eksikse kurulum durmaz (Breeze/Fusion'a düşülür).
for qv in $QT_STYLE_BUILT; do
    so="$LIBEXEC/qt$qv/styles/libnuxut-style.so"
    if [ -f "$so" ] && cmp -s "$so" "$CARGO_TARGET_DIR/qt-style-qt$qv/plugins/styles/libnuxut-style.so"; then
        note "Qt$qv stili doğrulandı: $so"
    else
        warn "Qt$qv stili kurulamadı ($so); Qt$qv uygulamaları Breeze/Fusion ile sürer."
    fi
done
[ -z "$QT_STYLE_FAILED" ] || warn "Nuxut Qt stili yok (Qt$(echo $QT_STYLE_FAILED | tr ' ' '/')): Qt uygulamaları Breeze/Fusion ile sürer; yukarıdaki uyarıya bakın."

# Ayarlar: uygulama girdileri ve ikon settings deposunun dist/ dizininden (Exec mutlak yola).
# nuxut-welcome.desktop gizli (NoDisplay): yalnızca Hoş geldin penceresinin (app-id nuxut-welcome)
# Ayarlar ikonuyla tanınması için.
for d in nuxut-settings nuxut-welcome; do
    if [ -f "$SRC/settings/dist/$d.desktop" ]; then
        sed "s#^Exec=nuxut-settings#Exec=$BIN/nuxut-settings#" "$SRC/settings/dist/$d.desktop" \
            > "$DATA/applications/$d.desktop"
    else
        warn "settings/dist/$d.desktop yok; uygulama girdisi kurulmadı."
    fi
done
if [ -f "$SRC/settings/dist/$SETTINGS_ICON" ]; then
    install -D -m 644 "$SRC/settings/dist/$SETTINGS_ICON" "$DATA/$SETTINGS_ICON"
    # Kullanıcı hicolor dizinine icon-theme.cache yazılmaz: bayat önbellek sonradan eklenen
    # ikonları gizler; önbelleksiz dizini GTK/Qt doğrudan tarar.
fi

# --- Hyprland ---------------------------------------------------------------------------
# ~/.config/hypr/nuxut.lua'yı (Hyprland ≥ 0.55; eski hyprland.conf kullananlarda nuxut.conf) kabuk
# üretir (başlatma, kısayollar, köşe/gölge/hareket) ve
# ilk açılışta Hyprland ayarında yoksa tek bir dahil etme satırı ekler (zaman damgalı yedekle;
# başka satıra dokunmaz). Burada yalnızca isteğe bağlı kapatma yapılır.
SHELL_TOML=$CONF/nuxut/shell.toml
if [ "$HYPR" = no ]; then
    if [ -f "$SHELL_TOML" ]; then
        if ! grep -qs '^\[compositor\]' "$SHELL_TOML"; then
            printf '\n[compositor]\nensure_source = false\n' >> "$SHELL_TOML"
            note "shell.toml: [compositor] ensure_source = false (kabuk hyprland.conf'a dokunmaz)."
        else
            note "shell.toml'da [compositor] var; ensure_source = false değerini elle verin."
        fi
    else
        mkdir -p "$CONF/nuxut"
        printf '[compositor]\nensure_source = false\n' > "$SHELL_TOML"
        note "shell.toml yazıldı: [compositor] ensure_source = false (kabuk hyprland.conf'a dokunmaz)."
    fi
    note "Elle eklemek için hyprland.lua'ya:  pcall(require, \"nuxut\")   (hyprland.conf: source = ~/.config/hypr/nuxut.conf)"
else
    # Ana ayarın devralınması (ADR-0007) yalnızca açık istekle (--hyprland-takeover); kurulum ve
    # güncelleme bunu kendiliğinden ASLA yapmaz (2026-10-16). İlk kurulumda kullanıcı onayını Hoş
    # geldin alır. Ana ayar Nuxut'u göstermiyorsa (başında 'nuxut: managed main config' işareti
    # yoksa) benzersiz yedek + Hyprland doğrulamasından geçmiş yeni ana dosya. Eski ayardan
    # taşınanlar (env/envd satırları; input, cursor, xwayland, render, opengl, device) ayrı
    # user-migrated.conf'a yazılır ve Nuxut ayarlarından önce okunur; exec-once satırları,
    # kısayollar ve pencere kuralları taşınmaz (raporda listelenir). Sonradan kendi satırlarınız
    # user.conf/user.lua'da, en son okunur. Geri: --hypr-restore. --dry-run planı yukarıda yazar.
    if [ "$HYPRLAND_TAKEOVER" = 1 ]; then
        if out=$("$BIN/nuxut-shell" hypr takeover </dev/null 2>&1); then
            note "Hyprland ana ayarı: $(printf '%s' "$out" | tr -d '\n' | sed 's/  */ /g')"
        else
            warn "Hyprland ana ayarı devralınamadı (eski dosya yerinde): $(printf '%s' "$out" | tr -d '\n' | sed 's/  */ /g')"
        fi
    fi
    note "Kabuk ilk açılışta ~/.config/hypr/nuxut.lua'yı üretir ve Hyprland ayarına tek dahil etme satırı ekler (yedekli; Geri al: nuxut-shell msg hypr undo)."
    note "Kapatmak için: sh install.sh --no-hyprland  ya da shell.toml [compositor] ensure_source = false"
fi

# --- Uyarılar ---------------------------------------------------------------------------
say "Kontroller"
# Çakışan bar/bildirim servisleri: devralınır (önce kayıt; --uninstall geri getirir). Bu
# oturumda yalnız Nuxut kabuğu çalışacaksa durdurulur, yoksa sonraki girişte etkin olur.
if [ "$TAKEOVER" = 1 ] && [ -f "$TAKEOVER_SH" ]; then
    now=""
    if [ "$FRESH_STOPPED" != none ] \
        || { [ "$RESTART" = 1 ] && [ -n "${WAYLAND_DISPLAY:-}" ] && { unit_active || shell_alive; }; }; then
        now=--now
    fi
    sh "$TAKEOVER_SH" apply $now 2>&1 | while IFS= read -r l; do note "devralma: $l"; done
else
    for p in dunst mako swaync waybar quickshell qs ags eww; do
        if pgrep -x "$p" >/dev/null 2>&1; then
            warn "$p çalışıyor: bildirim/tepsi adlarını tutabilir ya da ikinci bar çizer; Nuxut'tan önce kapatın (exec-once satırını da kaldırın)."
        fi
    done
fi
for c in pipewire wireplumber NetworkManager bluetoothd upowerd power-profiles-daemon; do
    pgrep -x "$c" >/dev/null 2>&1 || note "çalışmıyor: $c — ilgili hızlı ayar kartı gizli kalır (isteğe bağlı)."
done
command -v mpv >/dev/null 2>&1 || note "mpv yok — video duvar kağıdı için mpv paketi gerekir (isteğe bağlı)"
lsmod 2>/dev/null | grep -q '^i2c_dev' || note "harici monitör parlaklığı (DDC/CI) için i2c-dev çekirdek modülü ve ddcutil udev kuralı gerekir (isteğe bağlı)"
say "PATH"
path_check || true

say "Tamam — $(git -C "$SRC/shell" log -1 --format='shell %h') · $(git -C "$SRC/framework" log -1 --format='framework %h')"
if [ "$FRESH_DONE" = 1 ]; then
    if [ -n "$FRESH_BACKUP" ]; then
        say "Ayarlar sıfırlandı, yedek: $FRESH_BACKUP"
    else
        say "Ayarlar sıfırlandı (yedeklenecek Nuxut ayarı yoktu)"
    fi
elif [ "$INSTALL_KIND" = update ]; then
    note "Güncelleme: ayarlarınız korundu. Eski biçimi kabuk açılışta taşır ve onarır (önce yedek)."
    note "  durum: nuxut-shell msg config status · geri al: nuxut-shell msg config undo"
else
    note "İlk kurulum: kabuk ilk açıldığında Hoş geldin penceresi gelir."
fi
say "Kabuk yeniden başlatma"
restart_shell || warn "Kurulum tamam; yalnızca kabuğun yeniden başlatılması başarısız (yukarıya bakın)."
note "Ayarlar: nuxut-settings · Güncellemek için bu betiği yeniden çalıştırın."
