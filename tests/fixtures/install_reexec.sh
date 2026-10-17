#!/bin/sh
# install.sh kendini yeniden exec senaryosu (tests/it/install_reexec.rs çalıştırır): geçici HOME,
# sahte origin (yerel bare depolar), boşluklu $SRC ve tırnaklı --prefix, sahte cargo (derleme
# adımında 3 ile çıkar → betik orada durur; kullanıcının dosyalarına ulaşılmaz).
# Kullanım: install_reexec.sh <install.sh yolu>
set -eu
NEW=$1
# Paylaşımlı /tmp kaynak yolunda kabul edilmez (toctou-path-trust): test kendi hedef dizininde.
T=$(mktemp -d -p "${NUXUT_TEST_TMP:?NUXUT_TEST_TMP gerekli (paylaşımsız geçici dizin)}")
trap 'rm -rf "$T"' EXIT
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export HOME="$T/home"
mkdir -p "$HOME"
unset XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME NUXUT_INSTALL_REEXEC NUXUT_SRC 2>/dev/null || true
G="git -c user.name=t -c user.email=t@t -c init.defaultBranch=main -c advice.detachedHead=false"
R="$T/remote"
SRC="$T/src dizini"
mkdir -p "$R" "$SRC"
fail() { echo "BAŞARISIZ: $*"; exit 1; }

for repo in nuxutframework nuxutshell nuxutsettings nuxutcalendar; do
  $G init -q --bare "$R/$repo.git"
  $G clone -q "$R/$repo.git" "$T/w-$repo" 2>/dev/null
  if [ $repo = nuxutshell ]; then
    # Sürüm A: test edilen install.sh'nin kendisi.
    cp "$NEW" "$T/w-$repo/install.sh"
  else
    echo x > "$T/w-$repo/README"
  fi
  $G -C "$T/w-$repo" add -A
  $G -C "$T/w-$repo" commit -q -m A
  $G -C "$T/w-$repo" push -q origin main
done
# Önceki kurulum: $SRC'de A klonları.
for pair in framework:nuxutframework shell:nuxutshell settings:nuxutsettings calendar:nuxutcalendar; do
  $G clone -q "$R/${pair#*:}.git" "$SRC/${pair%%:*}"
done
# Origin'e sürüm B: argüman ayrıştırmasından sonra argümanlarını yazan işaret satırı.
awk '{print} /^BIN=\$PREFIX\/bin$/ && !d {print "echo \"SURUM-B prefix=[$PREFIX] src=[$SRC] branch=[$BRANCH] reexec=[$REEXECED] env=[${NUXUT_INSTALL_REEXEC:-}]\""; d=1}' \
  "$NEW" > "$T/w-nuxutshell/install.sh"
grep -q SURUM-B "$T/w-nuxutshell/install.sh" || fail "işaret satırı eklenemedi"
$G -C "$T/w-nuxutshell" commit -q -am B
$G -C "$T/w-nuxutshell" push -q origin main

# Sahte cargo: çağrıları sayar, 3 ile çıkar (derleme adımında betik durur).
mkdir -p "$T/bin"
printf '#!/bin/sh\necho "$@" >> "%s/cargo.log"\nexit 3\n' "$T" > "$T/bin/cargo"
chmod +x "$T/bin/cargo"
export NUXUT_GIT_BASE="$R" NUXUT_REPO_PREFIX=nuxut NUXUT_CARGO="$T/bin/cargo" NUXUT_BRANCH=main

run() { # $1 çıktı dosyası; kalanı argümanlar
  out=$1; shift
  set +e
  sh "$SRC/shell/install.sh" "$@" > "$out" 2>&1
  rc=$?
  set -e
  echo "$rc"
}

# 1) Güncelleme install.sh'yi değiştirir → tek yeniden exec, argümanlar korunur.
P="$T/pre fix'li"
rc=$(run "$T/o1" --no-deps --no-restart --prefix "$P" --src "$SRC" --keep-config --no-sharepicker)
[ "$rc" = 3 ] || { cat "$T/o1"; fail "1: çıkış $rc (derleme adımına ulaşmalıydı)"; }
n=$(grep -c "yeniden başlatılıyor" "$T/o1" || true)
[ "$n" = 1 ] || { cat "$T/o1"; fail "1: yeniden exec sayısı $n"; }
grep -qF "SURUM-B prefix=[$P] src=[$SRC] branch=[main] reexec=[1] env=[]" "$T/o1" || { cat "$T/o1"; fail "1: B sürümü argümanları/bekçi"; }
[ "$(wc -l < "$T/cargo.log")" = 1 ] || fail "1: cargo çağrı sayısı"
echo "1 tamam: tek yeniden exec, --src (boşluklu) ve NUXUT_BRANCH geçti, bekçi değişkeni sızmadı"

# 2) Değişiklik yok → yeniden exec yok.
rc=$(run "$T/o2" --no-deps --no-restart --src "$SRC" --keep-config)
[ "$rc" = 3 ] || { cat "$T/o2"; fail "2: çıkış $rc"; }
grep -q "yeniden başlatılıyor" "$T/o2" && { cat "$T/o2"; fail "2: gereksiz yeniden exec"; }
grep -qF "src=[$SRC] branch=[main] reexec=[]" "$T/o2" || fail "2: B doğrudan çalışmadı"
echo "2 tamam: güncel betikte yeniden exec yok"

# 3) Bekçi: origin'de yeni sürüm C var ama NUXUT_INSTALL_REEXEC=1 → exec yok (döngü olmaz).
echo "# C" >> "$T/w-nuxutshell/install.sh"
$G -C "$T/w-nuxutshell" commit -q -am C
$G -C "$T/w-nuxutshell" push -q origin main
set +e
NUXUT_INSTALL_REEXEC=1 sh "$SRC/shell/install.sh" --no-deps --no-restart --src "$SRC" > "$T/o3" 2>&1
rc=$?
set -e
[ "$rc" = 3 ] || { cat "$T/o3"; fail "3: çıkış $rc"; }
grep -q "yeniden başlatılıyor" "$T/o3" && { cat "$T/o3"; fail "3: bekçi çalışmadı"; }
grep -q "# C" "$SRC/shell/install.sh" || fail "3: kaynak C'ye güncellenmedi"
echo "3 tamam: bekçi varken yeniden exec yok"

echo HEPSİ TAMAM
