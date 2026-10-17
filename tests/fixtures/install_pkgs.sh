#!/bin/sh
# install.sh paket seçimi (tests/it/install_pkgs.rs çalıştırır): `# >>> paket-seçimi` bloğu
# ayıklanıp sahte pacman/sudo ile çalıştırılır; gerçek pacman ve sudo'ya ulaşılmaz.
# Sahte pacman: -T yalnız FAKE_MISSING'dekileri eksik sayar (provides karşılanmış = listede yok),
# -Qq yalnız FAKE_INSTALLED'dakileri kurulu sayar; her çağrı pacman.log'a, sudo sudo.log'a yazılır.
# Kullanım: install_pkgs.sh <install.sh yolu>
set -eu
NEW=$1
T=$(mktemp -d -p "${NUXUT_TEST_TMP:?NUXUT_TEST_TMP gerekli (paylaşımsız geçici dizin)}")
trap 'rm -rf "$T"' EXIT
fail() { echo "BAŞARISIZ: $*"; exit 1; }

sed -n '/^# >>> paket-seçimi/,/^# <<< paket-seçimi/p' "$NEW" > "$T/block.sh"
grep -q '^pkg_install()' "$T/block.sh" || fail "paket-seçimi bloğu bulunamadı"

mkdir -p "$T/bin"
cat > "$T/bin/pacman" <<EOF
#!/bin/sh
echo "\$*" >> "$T/pacman.log"
has() { for x in \$2; do [ "\$x" = "\$1" ] && return 0; done; return 1; }
case \$1 in
  -T) shift; rc=0
      for p in "\$@"; do if has "\$p" "\${FAKE_MISSING:-}"; then echo "\$p"; rc=127; fi; done
      exit \$rc ;;
  -Qq) has "\$2" "\${FAKE_INSTALLED:-}" ;;
  -S) shift; for p in "\$@"; do case \$p in -*) ;; *) if has "\$p" "\${FAKE_FAIL:-}"; then echo "hata: \$p ile çakışıyor" >&2; exit 1; fi ;; esac; done; exit 0 ;;
  *) exit 1 ;;
esac
EOF
cat > "$T/bin/sudo" <<EOF
#!/bin/sh
echo "\$*" >> "$T/sudo.log"
exec "\$@"
EOF
chmod +x "$T/bin/pacman" "$T/bin/sudo"

# Senaryo: $1 ad, $2 FAKE_MISSING, $3 FAKE_INSTALLED, kalan: paketler. Çıktı $T/<ad>.out.
scenario() {
  name=$1; missing=$2; installed=$3; shift 3
  rm -f "$T/pacman.log" "$T/sudo.log"
  : > "$T/pacman.log"
  PATH="$T/bin:$PATH" FAKE_MISSING="$missing" FAKE_INSTALLED="$installed" \
    sh -c '. "$1"; shift; note() { printf "    %s\n" "$*"; }; pkg_install "$@"' x "$T/block.sh" "$@" \
    > "$T/$name.out" 2>&1 || fail "$name: pkg_install başarısız: $(cat "$T/$name.out")"
}
no_install() { # $1 ad: pacman -S ve sudo çağrılmadı
  [ ! -e "$T/sudo.log" ] || fail "$1: sudo çağrıldı: $(cat "$T/sudo.log")"
  if grep -q -- '^-S' "$T/pacman.log"; then fail "$1: pacman -S çağrıldı"; fi
}

# 1) Provides eden -git kurulu (adw-gtk-theme-git provides adw-gtk-theme): pacman -T eksik demez.
scenario provides "" "adw-gtk-theme-git" base-devel adw-gtk-theme
no_install provides
grep -q -- '-T base-devel adw-gtk-theme' "$T/pacman.log" || fail "provides: pacman -T çağrılmadı"
echo "1 tamam: provides eden varyant karşılanmış sayıldı"

# 2) Provides etmeyen -git kurulu: pacman -T eksik der, varyant görülür ve atlanır.
scenario variant "ttf-material-symbols-variable" "ttf-material-symbols-variable-git" \
  base-devel ttf-material-symbols-variable
no_install variant
grep -q 'ttf-material-symbols-variable: zaten ttf-material-symbols-variable-git kurulu, atlandı' "$T/variant.out" \
  || fail "variant: atlama notu yok: $(cat "$T/variant.out")"
echo "2 tamam: provides etmeyen -git varyantı atlandı"

# 2b) -bin varyantı da aynı kural.
scenario variant_bin "foo" "foo-bin" foo
no_install variant_bin
grep -q 'foo: zaten foo-bin kurulu, atlandı' "$T/variant_bin.out" || fail "variant_bin: atlama notu yok"
echo "2b tamam: -bin varyantı atlandı"

# 3) Hepsi kurulu: pacman -S ve sudo hiç çağrılmaz.
scenario all "" "" base-devel git adw-gtk-theme
no_install all
grep -q 'paketler tamam' "$T/all.out" || fail "all: 'paketler tamam' yok"
echo "3 tamam: hepsi kuruluyken sudo/pacman -S yok"

# 4) Gerçekten eksik olan yalnız o kurulur; karşılanmış ve varyantlı olanlar komutta yok.
scenario mixed "cmake ttf-material-symbols-variable" "ttf-material-symbols-variable-git" \
  base-devel cmake adw-gtk-theme ttf-material-symbols-variable
[ "$(cat "$T/sudo.log")" = "pacman -S --needed --noconfirm cmake" ] || fail "mixed: sudo: $(cat "$T/sudo.log")"
echo "4 tamam: yalnız eksik paket kuruldu"

# 5) Paket hatası kurulumu durdurmaz: toplu kurulum düşünce tek tek denenir, kurulamayan nedeniyle
#    yazılır ve PKG_FAILED'e girer, diğerleri kurulur, fonksiyon 0 döner.
rm -f "$T/sudo.log"; : > "$T/pacman.log"
PATH="$T/bin:$PATH" FAKE_MISSING="cmake adw-gtk-theme" FAKE_FAIL="adw-gtk-theme" FAKE_INSTALLED="" \
  sh -c '. "$1"; shift; note() { printf "    %s\n" "$*"; }; warn() { printf "!! %s\n" "$*"; }; pkg_install_soft "$@"; echo "rc=$? failed=[$PKG_FAILED]"' \
  x "$T/block.sh" base-devel cmake adw-gtk-theme > "$T/soft.out" 2>&1 || fail "soft: kabuk düştü: $(cat "$T/soft.out")"
grep -qF "rc=0 failed=[ adw-gtk-theme]" "$T/soft.out" || fail "soft: $(cat "$T/soft.out")"
grep -qF "adw-gtk-theme kurulamadı, atlandı: hata: adw-gtk-theme ile çakışıyor" "$T/soft.out" || fail "soft: neden yok: $(cat "$T/soft.out")"
grep -qx "pacman -S --needed --noconfirm cmake" "$T/sudo.log" || fail "soft: cmake tek başına kurulmadı: $(cat "$T/sudo.log")"
echo "5 tamam: paket hatasında kurulum sürdü, neden yazıldı"

# 6) Sıra: paketler kaynaklar çekilip betik kendini yeniden exec ettikten SONRA kurulur (eski bir
#    kopya paket adımında takılıp yeni betiğe ulaşamıyordu, 2026-10-17).
reexec=$(grep -n 'exec sh "$SRC/shell/install.sh"' "$NEW" | head -n 1 | cut -d: -f1)
pkgs=$(grep -n '^# --- Paketler' "$NEW" | cut -d: -f1)
srcs=$(grep -n '^# --- Kaynaklar' "$NEW" | cut -d: -f1)
[ -n "$reexec" ] && [ -n "$pkgs" ] && [ "$srcs" -lt "$reexec" ] && [ "$reexec" -lt "$pkgs" ] \
  || fail "sıra: kaynaklar=$srcs yeniden-exec=$reexec paketler=$pkgs"
echo "6 tamam: paket adımı yeniden exec'ten sonra"

echo "HEPSİ TAMAM"
