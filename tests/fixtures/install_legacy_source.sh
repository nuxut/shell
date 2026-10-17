#!/bin/sh
# install.sh kayıtsız eski kurulum senaryosu (tests/it/install_legacy_source.rs çalıştırır):
# klonlar bir aynayı gösteriyor, install.conf yok (önceki sürümler yazmıyordu). Ayna ve "GitHub"
# yerel bare depolardır; git `url.<yol>.insteadOf` ile yönlendirilir (ağa çıkılmaz). Terminal
# `script` ile (pty), terminalsizlik `setsid` ile sağlanır. Sahte cargo derleme adımında 3 ile çıkar.
# Kullanım: install_legacy_source.sh <install.sh yolu>
set -eu
NEW=$1
T=$(mktemp -d -p "${NUXUT_TEST_TMP:?NUXUT_TEST_TMP gerekli (paylaşımsız geçici dizin)}")
trap 'chmod -R u+w "$T" 2>/dev/null; rm -rf "$T"' EXIT
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export HOME="$T/home"
mkdir -p "$HOME"
unset XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME NUXUT_INSTALL_REEXEC NUXUT_SRC NUXUT_PREFIX \
  NUXUT_GIT_BASE NUXUT_REPO_PREFIX 2>/dev/null || true
G="git -c user.name=t -c user.email=t@t -c init.defaultBranch=main"
R="$T/mirror"
GH="$T/github"
SRC="$T/src"
CONF="$HOME/.local/state/nuxut/install.conf"
mkdir -p "$R" "$GH" "$SRC"
fail() { echo "BAŞARISIZ: $*"; exit 1; }

# Ayna (önekli adlar) ve ayrı geçmişli "GitHub" (öneksiz adlar).
for d in framework shell settings calendar; do
  for side in mirror github; do
    if [ $side = mirror ]; then bare="$R/nuxut$d.git"; else bare="$GH/$d.git"; fi
    $G init -q --bare "$bare"
    $G clone -q "$bare" "$T/w-$side-$d" 2>/dev/null
    if [ $d = shell ]; then cp "$NEW" "$T/w-$side-$d/install.sh"; else echo "$side" > "$T/w-$side-$d/README"; fi
    $G -C "$T/w-$side-$d" add -A
    $G -C "$T/w-$side-$d" commit -q -m "$side"
    $G -C "$T/w-$side-$d" push -q origin main
  done
done
export GIT_CONFIG_GLOBAL="$T/gitconfig"
git config --global url."$R/".insteadOf "ssh://u@mirror.test/owner/"
git config --global --add url."$R/".insteadOf "u@mirror.test:owner/"
git config --global url."$GH/".insteadOf "https://github.com/nuxut/"
git config --global url."$T/other/".insteadOf "ssh://other.test/x/"

# Eski kurulum: ayna klonları, origin scp biçiminde (eski betik ssh aynasından kurmuştu).
fresh_clones() {
  chmod -R u+w "$SRC" 2>/dev/null; rm -rf "$SRC"; mkdir -p "$SRC"; rm -f "$CONF"
  for d in framework shell settings calendar; do
    $G clone -q "$R/nuxut$d.git" "$SRC/$d"
    git -C "$SRC/$d" remote set-url origin "u@mirror.test:owner/nuxut$d.git"
  done
}

mkdir -p "$T/bin"
printf '#!/bin/sh\necho "$@" >> "%s/cargo.log"\nexit 3\n' "$T" > "$T/bin/cargo"
chmod +x "$T/bin/cargo"
export NUXUT_CARGO="$T/bin/cargo" NUXUT_BRANCH=main
ARGS="--no-deps --no-restart --keep-config --no-sharepicker --prefix $T/prefix --src $SRC"

notty() { # $1 çıktı → rc; denetim terminali yok
  set +e
  # shellcheck disable=SC2086
  setsid -w sh "$SRC/shell/install.sh" $ARGS > "$1" 2>&1 < /dev/null
  rc=$?; set -e; echo "$rc"
}
withtty() { # $1 cevap, $2 çıktı → rc; pty içinde, cevap terminalden okunur
  set +e
  printf '%s\n' "$1" | script -qec "sh '$SRC/shell/install.sh' $ARGS" /dev/null > "$2" 2>&1
  rc=$?; set -e; echo "$rc"
}

# 1) Terminal yok: açık iletiyle durur, kayıt yazılmaz, derleme başlamaz.
fresh_clones
rc=$(notty "$T/o1")
[ "$rc" = 1 ] || { cat "$T/o1"; fail "1: çıkış $rc"; }
grep -qF "terminalde çalıştırın" "$T/o1" || { cat "$T/o1"; fail "1: ileti yok"; }
grep -qF "NUXUT_GIT_BASE=ssh://u@mirror.test/owner NUXUT_REPO_PREFIX=nuxut" "$T/o1" || { cat "$T/o1"; fail "1: kaynak önerisi yok"; }
if grep -qF "Kaldırın" "$T/o1"; then fail "1: 'Kaldırın' denmemeli"; fi
[ ! -e "$CONF" ] || fail "1: kayıt yazıldı"
[ ! -e "$T/cargo.log" ] || fail "1: derleme başladı"
echo "1 tamam: terminalsiz eski kurulum açık iletiyle durdu"

# 2) Terminal + evet: aynı kaynakla sürer, kayıt (0600) yazılır, derlemeye ulaşılır.
rc=$(withtty e "$T/o2")
[ "$rc" = 3 ] || { cat "$T/o2"; fail "2: çıkış $rc"; }
grep -qF "Mevcut kurulum ssh://u@mirror.test/owner kaynağından güncelleniyor" "$T/o2" || { cat "$T/o2"; fail "2: soru yok"; }
grep -qx "base=ssh://u@mirror.test/owner" "$CONF" && grep -qx "prefix=nuxut" "$CONF" || { cat "$CONF"; fail "2: kayıt yanlış"; }
[ "$(stat -c %a "$CONF")" = 600 ] || fail "2: kayıt izni"
[ "$(git -C "$SRC/shell" config --get remote.origin.url)" = "u@mirror.test:owner/nuxutshell.git" ] || fail "2: origin değişti"
echo "2 tamam: onayla aynı kaynakla sürdü, kayıt yazıldı"

# 2b) Sonraki çalıştırma terminalsiz de kayıttan sürer.
rc=$(notty "$T/o2b")
[ "$rc" = 3 ] || { cat "$T/o2b"; fail "2b: çıkış $rc"; }
grep -qF "kayıtlı kaynak" "$T/o2b" || { cat "$T/o2b"; fail "2b: kayıt kullanılmadı"; }
echo "2b tamam: kayıttan sürdü"

# 3) Terminal + hayır: durur, hiçbir şey değişmez.
fresh_clones
rc=$(withtty h "$T/o3")
[ "$rc" = 1 ] || { cat "$T/o3"; fail "3: çıkış $rc"; }
grep -qF "durduruldu" "$T/o3" || { cat "$T/o3"; fail "3: ileti yok"; }
[ ! -e "$CONF" ] || fail "3: kayıt yazıldı"
echo "3 tamam: hayır cevabıyla durdu"

# 4) Karışık kaynaklar: terminal olsa da reddedilir.
fresh_clones
git -C "$SRC/settings" remote set-url origin "ssh://other.test/x/nuxutsettings.git"
rc=$(withtty e "$T/o4")
[ "$rc" = 1 ] || { cat "$T/o4"; fail "4: çıkış $rc"; }
grep -qF "farklı kaynakları" "$T/o4" || { cat "$T/o4"; fail "4: ileti yok"; }
[ ! -e "$CONF" ] || fail "4: kayıt yazıldı"
echo "4 tamam: karışık kaynaklar reddedildi"

# 5) GitHub'a geç, yerel değişiklik var: geçilmez, origin aynı kalır.
fresh_clones
echo yerel >> "$SRC/calendar/README"
rc=$(withtty g "$T/o5")
[ "$rc" = 1 ] || { cat "$T/o5"; fail "5: çıkış $rc"; }
grep -qF "yerel değişiklik var; GitHub'a geçilmedi" "$T/o5" || { cat "$T/o5"; fail "5: ileti yok"; }
[ "$(git -C "$SRC/calendar" config --get remote.origin.url)" = "u@mirror.test:owner/nuxutcalendar.git" ] || fail "5: origin değişti"
echo "5 tamam: kirli klonla GitHub'a geçilmedi"

# 6) GitHub'a geç, temiz klonlar: ayrı geçmiş → eski klon yedekte, GitHub'dan yeniden indirildi.
fresh_clones
rc=$(withtty g "$T/o6")
[ "$rc" = 3 ] || { cat "$T/o6"; fail "6: çıkış $rc"; }
for d in framework shell settings calendar; do
  [ "$(git -C "$SRC/$d" config --get remote.origin.url)" = "https://github.com/nuxut/$d.git" ] || fail "6: $d origin"
  ls -d "$SRC/$d".nuxut-yedek-* >/dev/null 2>&1 || fail "6: $d yedeği yok"
done
[ "$(cat "$SRC/framework/README")" = github ] || fail "6: GitHub içeriği yok"
[ ! -e "$CONF" ] || fail "6: GitHub için kayıt yazılmamalı"
echo "6 tamam: GitHub'a geçildi, eski klonlar yedekte"

# 7) GitHub'a geçilmiş kurulum terminalsiz sürer (soru yok).
rc=$(notty "$T/o7")
[ "$rc" = 3 ] || { cat "$T/o7"; fail "7: çıkış $rc"; }
if grep -qF "Mevcut kurulum" "$T/o7"; then fail "7: yine soruldu"; fi
echo "7 tamam: GitHub kurulumu sorusuz sürdü"

echo "HEPSİ TAMAM"
