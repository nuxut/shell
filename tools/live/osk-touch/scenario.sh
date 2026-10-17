#!/bin/sh
# Ekran klavyesi dokunma uçtan uca (docs/design/osk.md "Dokunma"). YALNIZ yalıtılmış a11y ortamında
# (başsız sway + ayrı oturum ve a11y bus'ı; tuş kutuları AT-SPI'dan okunur):
#
#   cargo-gate cargo build --features test-touch && cargo-gate cargo build --example at_probe
#   flock -o <canlı-test-kilidi> <framework>/tools/a11y-e2e.sh sh tools/live/osk-touch/scenario.sh ÇIKTI
#
# Dokunma framework'ün sentetik dokunmasıyla (`test-touch`, `NUXUT_TEST_TOUCH` FIFO'su; wl_touch
# işleyicisiyle aynı yol). Yazılanlar kendi `kitty --class nuxut-test` penceremizdeki tuzağa düşer.
set -eu
[ "${NUXUT_A11Y_E2E:-}" = 1 ] || { echo "yalnız a11y-e2e.sh içinde" >&2; exit 2; }
OUT=$1
root=$(cd "$(dirname "$0")/../../.." && pwd)
SHELL_BIN=${NUXUT_TEST_SHELL:-$root/target/debug/nuxut-shell}
PROBE=${NUXUT_AT_PROBE:-$root/target/debug/examples/at_probe}
mkdir -p "$OUT"
export HOME=$OUT/home LANG=tr_TR.UTF-8 NUXUT_SHELL_INSTANCE=test-osk-touch
mkdir -p "$HOME"
unset HYPRLAND_INSTANCE_SIGNATURE
grep -a -q NUXUT_TEST_TOUCH "$SHELL_BIN" || { echo "kabuk test-touch özelliğiyle derlenmemiş" >&2; exit 2; }
busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true

pids=""
cleanup() {
    set +e
    for p in $pids; do kill "$p" 2> /dev/null; done
    sleep 0.5
    for p in $pids; do kill -0 "$p" 2> /dev/null && kill -9 "$p" 2> /dev/null; done
}
trap cleanup EXIT INT TERM HUP
fail() {
    echo "BAŞARISIZ: $*" >&2
    exit 1
}
wait_for() {
    i=0
    while [ "$i" -lt $(($3 * 20)) ]; do
        grep -q -- "$2" "$1" 2> /dev/null && return 0
        sleep 0.05
        i=$((i + 1))
    done
    return 1
}

# Girdi tuzağı: odaktaki kendi penceremiz.
keys=$OUT/keys
: > "$keys"
(exec kitty --class nuxut-test -o confirm_os_window_close=0 sh "$root/lock/tools/live/capture.sh" "$keys" > "$OUT/kitty.log" 2>&1) &
pids="$pids $!"
i=0
until swaymsg -t get_tree | jq -e '.. | objects | select(.app_id? == "nuxut-test" and .focused?)' > /dev/null; do
    [ "$i" -lt 100 ] || fail "nuxut-test penceresi odak almadı"
    sleep 0.1
    i=$((i + 1))
done
sleep 0.5

fifo=$OUT/touch.fifo
rm -f "$fifo"
mkfifo "$fifo"
(exec env NUXUT_TEST_TOUCH="$fifo" RUST_LOG=info "$SHELL_BIN" > "$OUT/shell.log" 2>&1) &
shell_pid=$!
pids="$pids $shell_pid"
wait_for "$OUT/shell.log" "SENTETİK dokunma açık" 10 || fail "sentetik dokunma açılmadı"
sleep 3
kill -0 "$shell_pid" || fail "kabuk düştü"
# Kendi örneğimiz mi: ping. Değilse hiçbir komut gönderilmez.
"$SHELL_BIN" msg ping > /dev/null || fail "test örneğine ping yok"
"$SHELL_BIN" msg osk show > /dev/null
sleep 1.5
"$SHELL_BIN" msg osk status | tee "$OUT/status0.json" | grep -q '"visible":true' || fail "klavye açılmadı"

# Tuş kutuları (yüzey koordinatı) AT-SPI'dan.
"$PROBE" "" --wait 10 --extents a --extents s --extents d --extents f --extents g --extents h \
    --extents "Geri sil" > "$OUT/extents.txt" 2>&1 || true
cat "$OUT/extents.txt"
center() {
    line=$(grep "^extents $1: " "$OUT/extents.txt") || fail "$1 kutusu yok"
    set -- ${line#*: }
    echo "$(($1 + $3 / 2)) $(($2 + $4 / 2))"
}
t() { echo "$*" > "$fifo"; }
tap() {
    t "down 0 nuxut-osk $(center "$1")"
    sleep 0.08
    t "up 0"
    sleep 0.3
}

# 1) Kısa dokunuş kalkışta yazar.
tap a
# 2) Kaydırarak düzeltme: s'de basılan parmak d'ye kayar, yalnız d yazılır.
t "down 0 nuxut-osk $(center s)"
sleep 0.08
t "motion 0 $(center d)"
sleep 0.08
t "up 0"
sleep 0.3
# 3) İki başparmak: f ve g birlikte basılı, kalkış sırasıyla yazılır.
t "down 0 nuxut-osk $(center f)"
sleep 0.05
t "down 1 nuxut-osk $(center g)"
sleep 0.08
t "up 0"
sleep 0.05
t "up 1"
sleep 0.3
# 4) İptal: h basılıyken compositor iptali; hiçbir şey yazılmaz.
t "down 0 nuxut-osk $(center h)"
sleep 0.08
t "cancel"
sleep 0.3
# 5) Basılı tutma: Geri sil 1.6 sn (TOUCH_HOLD + istemci tekrarı) birden çok siler.
t "down 0 nuxut-osk $(center "Geri sil")"
sleep 1.6
t "up 0"
sleep 0.5

printf 'yakalanan: '
od -An -c "$keys" | tr -s ' '
got=$(tr -d '\177' < "$keys")
dels=$(tr -cd '\177' < "$keys" | wc -c)
[ "$got" = "adfg" ] || fail "yazılan '$got', beklenen 'adfg'"
[ "$dels" -ge 3 ] || fail "basılı tutulan Geri sil $dels kez sildi (en az 3 beklenir)"

# 6) Tutamaçtan dokunmayla sürükleme (sahiplenilmez, işaretçi öykünmesi): yukarı çekince yüzer.
# Tutamaç ilk tuş satırının hemen üstünde (osk_view::layout: satırlar frame.y + tutamaç yüksekliğinden).
set -- $(center g)
hx=$1
top=$(grep "^extents Geri sil: " "$OUT/extents.txt" | awk '{print $5}')
hy=$((top - 8))
t "down 0 nuxut-osk $hx $hy"
sleep 0.15
for dy in 20 60 120 200; do
    t "motion 0 $hx $((hy - dy))"
    sleep 0.05
done
t "up 0"
sleep 1.2
"$SHELL_BIN" msg osk status | tee "$OUT/status1.json"
echo
grep -q '"placement":"floating"' "$OUT/status1.json" || fail "tutamaç dokunmayla sürüklenmedi"
grim -o HEADLESS-1 "$OUT/osk-floating.png" || true
grep -q -i "panicked" "$OUT/shell.log" && fail "kabuk paniğe düştü"
echo "osk-touch: geçti"
