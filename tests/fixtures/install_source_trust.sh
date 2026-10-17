#!/bin/sh
# install.sh kaynak güveni senaryosu (tests/it/install_source_trust.rs çalıştırır): geçici HOME,
# yerel bare "origin"ler, sahte cargo (derleme adımında 3 ile çıkar → betik orada durur). Kaynak
# yeri/kökeni güvenilmezse betik derlemeye ulaşmadan 1 ile çıkmalı (güvenlik incelemesi 2026-10-16).
# Kullanım: install_source_trust.sh <install.sh yolu>
set -eu
NEW=$1
# Paylaşımlı /tmp kaynak yolunda kabul edilmez (toctou-path-trust): test kendi hedef dizininde.
T=$(mktemp -d -p "${NUXUT_TEST_TMP:?NUXUT_TEST_TMP gerekli (paylaşımsız geçici dizin)}")
trap 'chmod -R u+w "$T" 2>/dev/null; rm -rf "$T"' EXIT
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}" CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export HOME="$T/home"
mkdir -p "$HOME"
unset XDG_CONFIG_HOME XDG_DATA_HOME XDG_STATE_HOME NUXUT_INSTALL_REEXEC NUXUT_SRC NUXUT_PREFIX 2>/dev/null || true
G="git -c user.name=t -c user.email=t@t -c init.defaultBranch=main"
R="$T/remote"
SRC="$T/src"
mkdir -p "$R" "$SRC"
fail() { echo "BAŞARISIZ: $*"; exit 1; }

for repo in nuxutframework nuxutshell nuxutsettings nuxutcalendar; do
  $G init -q --bare "$R/$repo.git"
  $G clone -q "$R/$repo.git" "$T/w-$repo" 2>/dev/null
  if [ $repo = nuxutshell ]; then cp "$NEW" "$T/w-$repo/install.sh"; else echo x > "$T/w-$repo/README"; fi
  $G -C "$T/w-$repo" add -A
  $G -C "$T/w-$repo" commit -q -m A
  $G -C "$T/w-$repo" push -q origin main
done
for pair in framework:nuxutframework shell:nuxutshell settings:nuxutsettings calendar:nuxutcalendar; do
  $G clone -q "$R/${pair#*:}.git" "$SRC/${pair%%:*}"
done
$G clone -q --bare "$R/nuxutsettings.git" "$T/rogue/nuxutsettings.git"

mkdir -p "$T/bin"
printf '#!/bin/sh\necho "$@" >> "%s/cargo.log"\nexit 3\n' "$T" > "$T/bin/cargo"
chmod +x "$T/bin/cargo"
export NUXUT_GIT_BASE="$R" NUXUT_REPO_PREFIX=nuxut NUXUT_CARGO="$T/bin/cargo" NUXUT_BRANCH=main
ARGS="--no-deps --no-restart --keep-config --no-sharepicker --prefix $T/prefix"

run() { # $1 çıktı; kalanı install.sh argümanları
  out=$1; shift
  set +e
  sh "$NEW" "$@" > "$out" 2>&1
  rc=$?
  set -e
  echo "$rc"
}
cargo_calls() { [ -f "$T/cargo.log" ] && wc -l < "$T/cargo.log" || echo 0; }
expect_refused() { # $1 adım, $2 çıktı, $3 rc, $4 beklenen ileti
  [ "$3" = 1 ] || { cat "$2"; fail "$1: çıkış $3 (1 bekleniyordu)"; }
  grep -qF "$4" "$2" || { cat "$2"; fail "$1: ileti yok: $4"; }
  [ "$(cargo_calls)" = "$BASE_CALLS" ] || fail "$1: derleme başladı"
}

# 0) Güvenilir kaynak: derleme adımına ulaşılır.
rc=$(run "$T/o0" $ARGS --src "$SRC")
[ "$rc" = 3 ] || { cat "$T/o0"; fail "0: çıkış $rc"; }
BASE_CALLS=$(cargo_calls)
echo "0 tamam: güvenilir kaynak derlemeye ulaştı"

# 1) Var olan klonun origin'i başka depo.
git -C "$SRC/settings" remote set-url origin "$T/rogue/nuxutsettings.git"
rc=$(run "$T/o1" $ARGS --src "$SRC")
expect_refused 1 "$T/o1" "$rc" "beklenen depodan değil"
git -C "$SRC/settings" remote set-url origin "$R/nuxutsettings.git"
echo "1 tamam: yabancı origin reddedildi"

# 2) Uzak adres https/ssh/mutlak yol dışında.
for base in http://example.invalid/hakan git://example.invalid/hakan relative/dir; do
  rc=$(NUXUT_GIT_BASE=$base run "$T/o2" $ARGS --src "$SRC")
  expect_refused "2 ($base)" "$T/o2" "$rc" "NUXUT_GIT_BASE yalnız"
done
echo "2 tamam: http/git/göreli uzak adres reddedildi"

# 3) Göreli kaynak dizini (çalışma dizinine bağlı olurdu).
rc=$(cd "$T" && run "$T/o3" $ARGS --src src)
expect_refused 3 "$T/o3" "$rc" "mutlak yol olmalı"
echo "3 tamam: göreli --src reddedildi"

# 4) Başkalarının yazabildiği kaynak dizini ya da klon.
chmod 777 "$SRC/shell"
rc=$(run "$T/o4" $ARGS --src "$SRC")
chmod 755 "$SRC/shell"
expect_refused "4a" "$T/o4" "$rc" "başkaları yazabiliyor"
chmod 775 "$SRC"
rc=$(run "$T/o4" $ARGS --src "$SRC")
chmod 755 "$SRC"
expect_refused "4b" "$T/o4" "$rc" "başka bir kullanıcının değiştirebileceği"
echo "4 tamam: grup/başkaları yazabilen dizin reddedildi"

# 5) Üst dizin herkese yazılabilir ama yapışkan değil (başkası kaynağı yeniden adlandırabilir).
mkdir -p "$T/open"
chmod 777 "$T/open"
rc=$(run "$T/o5" $ARGS --src "$T/open/nuxut")
expect_refused 5 "$T/o5" "$rc" "üst dizini"
[ ! -e "$T/open/nuxut" ] || fail "5: dizin oluşturuldu"
chmod 1777 "$T/open"
rc=$(run "$T/o5b" $ARGS --src "$T/open/nuxut")
expect_refused 5b "$T/o5b" "$rc" "üst dizini"
[ ! -e "$T/open/nuxut" ] || fail "5b: yapışkan paylaşımlı dizinde dizin oluşturuldu"
echo "5 tamam: herkese açık üst dizin (yapışkan da) reddedildi"

# 6) Borudan çalışma (curl | sh): $0 "sh"; çalışma dizinindeki "sh" dosyası parmak izi sayılmaz,
#    yeniden exec olmaz.
mkdir -p "$T/cwd"
echo 'echo SAHTE' > "$T/cwd/sh"
set +e
(cd "$T/cwd" && sh -s -- $ARGS --src "$SRC" < "$NEW") > "$T/o6" 2>&1
rc=$?
set -e
[ "$rc" = 3 ] || { cat "$T/o6"; fail "6: çıkış $rc"; }
grep -q "yeniden başlatılıyor" "$T/o6" && { cat "$T/o6"; fail "6: borudan çalışmada yeniden exec"; }
echo "6 tamam: borudan çalışmada çalışma dizini kullanılmadı"
BASE_CALLS=$(cargo_calls)

# 7) path-trust-bypass: kendi dizinimizdeki bağ, başkasının yazabildiği dizindeki bir alt dizine
#    gidiyor. Bağın kendisi ve hedefi bize ait; denetim kanonik yolda yapılmalı.
mkdir -p "$T/evil/b"
chmod 777 "$T/evil"
ln -s "$T/evil/b" "$HOME/link"
rc=$(run "$T/o7" $ARGS --src "$HOME/link/nuxut")
expect_refused 7 "$T/o7" "$rc" "üst dizini"
[ ! -e "$T/evil/b/nuxut" ] || fail "7: bağın hedefinde dizin oluşturuldu"
echo "7 tamam: sembolik bağ üzerinden güvenilmeyen dizin reddedildi"

# 8) `..` ve sondaki `/`: kanonik yol denetlenir.
rc=$(run "$T/o8" $ARGS --src "$SRC/../evil/nuxut")
expect_refused "8a" "$T/o8" "$rc" "üst dizini"
rc=$(run "$T/o8b" $ARGS --src "$SRC/../src/")
[ "$rc" = 3 ] || { cat "$T/o8b"; fail "8b: kanonik olarak güvenilir yol reddedildi ($rc)"; }
BASE_CALLS=$(cargo_calls)
echo "8 tamam: '..' ile kaçış reddedildi, sondaki '/' ve geri dönen '..' kabul edildi"

# 9) Güvenilmeyen kurulum öneki (ikililer oturumda çalışır).
rc=$(run "$T/o9" --no-deps --no-restart --keep-config --src "$SRC" --prefix "$T/evil/p")
expect_refused 9 "$T/o9" "$rc" "Kurulum öneki"
echo "9 tamam: başkasının yazabildiği önek reddedildi"

# 10) supply-chain: dal adı seçenek olamaz (git fetch --upload-pack komut çalıştırırdı).
rc=$(NUXUT_BRANCH="--upload-pack=touch $T/pwned" run "$T/o10" $ARGS --src "$SRC")
expect_refused 10 "$T/o10" "$rc" "Geçersiz dal adı"
[ ! -e "$T/pwned" ] || fail "10: komut çalıştı"
echo "10 tamam: seçenek biçimli dal adı reddedildi"

# 11) supply-chain: her derleme --locked (Cargo.lock dışında crate çekilmez).
[ -s "$T/cargo.log" ] || fail "11: cargo hiç çağrılmadı"
if grep -v -- '--locked' "$T/cargo.log"; then fail "11: --locked olmayan cargo çağrısı"; fi
n=$(grep -c 'CARGO build --locked' "$NEW")
[ "$(grep -c 'CARGO build' "$NEW")" = "$n" ] || fail "11: install.sh'de --locked olmayan derleme satırı"
echo "11 tamam: bütün derlemeler --locked"

# 12) supply-chain: betik AUR yardımcısı (paru/yay) çağırmaz (yalnız pacman -Qi ile algılar), curl/wget
#     ile betik indirip çalıştırmaz; git yalnız https/ssh/file taşımalarını kullanır.
if grep -nE '^[^#]*(paru|yay|\$AUR[A-Z]*)[[:space:]]+-S' "$NEW"; then fail "12: AUR yardımcısı çağrısı"; fi
if grep -nE '^[^#]*(curl|wget)[^|]*\|[[:space:]]*(ba|da|z)?sh' "$NEW"; then fail "12: indirilen betik çalıştırılıyor"; fi
grep -q '^export GIT_ALLOW_PROTOCOL=https:ssh:file$' "$NEW" || fail "12: GIT_ALLOW_PROTOCOL yok"
echo "12 tamam: AUR yardımcısı çağrılmaz, curl|sh yok, git taşımaları sınırlı"

# 13) supply-chain: sabitleme. Her depoda t1 etiketi; sonra framework'e yeni commit. --ref t1 ile
#     framework t1'de kalır, dal ucuna gitmez; çekilen commit'ler çıktıda ve kurulum kaydında.
for repo in nuxutframework nuxutshell nuxutsettings nuxutcalendar; do
  $G -C "$T/w-$repo" tag t1
  $G -C "$T/w-$repo" push -q origin t1
done
T1=$(git -C "$T/w-nuxutframework" rev-parse HEAD)
echo y >> "$T/w-nuxutframework/README"
$G -C "$T/w-nuxutframework" commit -q -am C
$G -C "$T/w-nuxutframework" push -q origin main
TIP=$(git -C "$T/w-nuxutframework" rev-parse HEAD)
rc=$(run "$T/o13" $ARGS --src "$SRC" --ref t1)
[ "$rc" = 3 ] || { cat "$T/o13"; fail "13: çıkış $rc"; }
[ "$(git -C "$SRC/framework" rev-parse HEAD)" = "$T1" ] || fail "13: framework t1'de değil"
grep -qF "framework: $T1 (sabit: t1)" "$T/o13" || { cat "$T/o13"; fail "13: commit çıktıda yok"; }
REC="$HOME/.local/state/nuxut/install-sources"
grep -qx "framework nuxutframework $T1 t1" "$REC" || { cat "$REC"; fail "13: kurulum kaydı"; }
grep -qx "durum kaynak" "$REC" || fail "13: kayıt durumu (derleme başarısızken kuruldu denmemeli)"
# Sabitlemesiz: dal ucu, commit yine yazılır.
rc=$(run "$T/o13b" $ARGS --src "$SRC")
[ "$rc" = 3 ] || { cat "$T/o13b"; fail "13b: çıkış $rc"; }
[ "$(git -C "$SRC/framework" rev-parse HEAD)" = "$TIP" ] || fail "13b: dal ucuna dönmedi"
grep -qx "framework nuxutframework $TIP -" "$REC" || { cat "$REC"; fail "13b: kayıt"; }
BASE_CALLS=$(cargo_calls)
echo "13 tamam: --ref etikete sabitler, commit'ler çıktıda ve kayıtta"

# 14) Sabitleme referansı seçenek olamaz; olmayan referans derlemeden önce durur.
rc=$(NUXUT_REF="--upload-pack=touch $T/pwned2" run "$T/o14" $ARGS --src "$SRC")
expect_refused 14a "$T/o14" "$rc" "Geçersiz sabitleme"
[ ! -e "$T/pwned2" ] || fail "14a: komut çalıştı"
rc=$(NUXUT_REF_SETTINGS=yok-boyle-bir-etiket run "$T/o14b" $ARGS --src "$SRC")
expect_refused 14b "$T/o14b" "$rc" "bulunamadı"
echo "14 tamam: geçersiz ve olmayan sabitleme reddedildi"

# 15) Doğrulanmamış AUR paketine güven yok (2026-10-16): Nuxut paketi kuruluysa yalnız `pacman -Qi`
#     ile algılanır, betik kaynaktan kurmadan 0 ile çıkar; AUR yardımcısı hiç çağrılmaz.
mkdir -p "$T/fakebin"
printf '#!/bin/sh\necho "$@" >> "%s/pacman.log"\n[ "$1" = -Qi ] && [ "$2" = nuxut-git ] && exit 0\nexit 1\n' "$T" > "$T/fakebin/pacman"
for h in paru yay; do printf '#!/bin/sh\ntouch "%s/%s-called"\n' "$T" "$h" > "$T/fakebin/$h"; done
chmod +x "$T/fakebin/"*
rc=$(PATH="$T/fakebin:$PATH" run "$T/o15" --no-restart --keep-config --no-sharepicker --prefix "$T/prefix" --src "$SRC")
[ "$rc" = 0 ] || { cat "$T/o15"; fail "15: çıkış $rc (0 bekleniyordu)"; }
grep -q "paketiyle kurulu" "$T/o15" || { cat "$T/o15"; fail "15: paket iletisi yok"; }
[ "$(cargo_calls)" = "$BASE_CALLS" ] || fail "15: paket kuruluyken derleme başladı"
[ ! -e "$T/paru-called" ] && [ ! -e "$T/yay-called" ] || fail "15: AUR yardımcısı çağrıldı"
grep -q -- '-Qi nuxut-git' "$T/pacman.log" || fail "15: pacman -Qi ile algılanmadı"
if grep -vE '^-Q' "$T/pacman.log"; then fail "15: pacman sorgu dışında çağrıldı"; fi
echo "15 tamam: kurulu paket yalnız algılandı, AUR yardımcısı ve derleme yok"

# 16) Ayna hatırlama: ortamla verilen NUXUT_GIT_BASE/NUXUT_REPO_PREFIX install.conf'a (0600) yazılır;
#     ortam yokken oradan okunur ve origin denetimi ona göre yapılır. Güvenilmeyen kayıt reddedilir.
CONF="$HOME/.local/state/nuxut/install.conf"
[ -f "$CONF" ] || fail "16: install.conf yazılmadı"
[ "$(stat -c %a "$CONF")" = 600 ] || fail "16: install.conf 0600 değil"
grep -qx "base=$R" "$CONF" && grep -qx "prefix=nuxut" "$CONF" || { cat "$CONF"; fail "16: kayıt içeriği"; }
norun() { out=$1; shift; set +e; env -u NUXUT_GIT_BASE -u NUXUT_REPO_PREFIX sh "$NEW" "$@" > "$out" 2>&1; rc=$?; set -e; echo "$rc"; }
rc=$(norun "$T/o16" $ARGS --src "$SRC")
[ "$rc" = 3 ] || { cat "$T/o16"; fail "16a: kayıtla çıkış $rc (3 bekleniyordu)"; }
grep -q "kayıtlı kaynak: $R (önek: nuxut)" "$T/o16" || { cat "$T/o16"; fail "16a: kayıtlı kaynak gösterilmedi"; }
grep -q "Kaynak aynası kaydedildi: $R" "$T/o0" || { cat "$T/o0"; fail "16: ilk kayıt gösterilmedi"; }
BASE_CALLS=$(cargo_calls)
chmod 664 "$CONF"
rc=$(norun "$T/o16b" $ARGS --src "$SRC")
expect_refused "16b" "$T/o16b" "$rc" "grup/başkaları yazabiliyor"
chmod 600 "$CONF"
cp "$CONF" "$T/conf.bak"
printf 'base=%s\nprefix=nuxut\n' "$T/rogue" > "$CONF"
rc=$(norun "$T/o16c" $ARGS --src "$SRC")
expect_refused "16c" "$T/o16c" "$rc" "beklenen depodan değil"
mv "$CONF" "$T/conf.real"; ln -s "$T/conf.bak" "$CONF"
rc=$(norun "$T/o16d" $ARGS --src "$SRC")
expect_refused "16d" "$T/o16d" "$rc" "sembolik bağ"
rm "$CONF"; mv "$T/conf.bak" "$CONF"; rm -f "$T/conf.real"
# Kayıttaki değerler ortamdakiyle aynı doğrulamadan geçer (untrusted persisted source).
cp "$CONF" "$T/conf.ok"
for bad in "base=http://example.invalid/x|NUXUT_GIT_BASE yalnız" "base=-ccore.sshCommand=x|NUXUT_GIT_BASE yalnız" "base=relative/dir|NUXUT_GIT_BASE yalnız" "base=/tmp/a b|boşluk" "prefix=-x|depo öneki" "prefix=Bad/x|depo öneki"; do
  printf '%s\n' "${bad%%|*}" > "$CONF"; chmod 600 "$CONF"
  rc=$(norun "$T/o16e" $ARGS --src "$SRC")
  expect_refused "16e (${bad%%|*})" "$T/o16e" "$rc" "${bad#*|}"
done
cp "$T/conf.ok" "$CONF"; chmod 600 "$CONF"
grep -q 'goksu' "$NEW" && fail "16: betikte özel adres var"
echo "16 tamam: ayna 0600 kayda yazıldı ve gösterildi, kayıttan okundu (kayıtlı kaynak çıktıda); yazılabilir/yabancı/bağ/geçersiz değerli kayıt reddedildi"

echo HEPSİ TAMAM
