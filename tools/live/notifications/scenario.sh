#!/bin/sh
# run.sh'ın iç adımı: özel dbus-run-session bus'ında. Doğrudan çalıştırma.
#   scenario.sh ÇIKTI_DİZİNİ KÖK
set -eu

OUT=$1
root=$2
case ${DBUS_SESSION_BUS_ADDRESS:-} in
    unix:path=/tmp/* | unix:abstract=* | unix:path=$XDG_RUNTIME_DIR/*) ;;
    *) echo "özel oturum bus'ı yok" >&2; exit 2 ;;
esac
export DBUS_SYSTEM_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS
export NUXUT_SHELL_INSTANCE=test-notify
SHELL_BIN=${NUXUT_TEST_SHELL:-$root/target/debug/nuxut-shell}
EX=${NUXUT_TEST_EXAMPLES:-$root/target/debug/examples}

pids=""
rec=""
cleanup() {
    set +e
    [ -n "$rec" ] && kill -INT "$rec" 2> /dev/null
    for p in $pids; do kill "$p" 2> /dev/null; done
    sleep 0.5
    for p in $pids; do kill -0 "$p" 2> /dev/null && kill -9 "$p" 2> /dev/null; done
    [ -n "${hypr_dir:-}" ] && rm -rf "$hypr_dir"
}
trap cleanup EXIT INT TERM HUP

# wait_for DOSYA DESEN SANİYE
wait_for() {
    i=0
    while [ "$i" -lt $(($3 * 20)) ]; do
        grep -q -- "$2" "$1" 2> /dev/null && return 0
        sleep 0.05
        i=$((i + 1))
    done
    return 1
}

# Sahte Hyprland (kullanıcınınkinden farklı imza).
sig=nuxut-test-notif-$$
hypr_dir=$XDG_RUNTIME_DIR/hypr/$sig
(exec "$EX/fake_hyprland" "$hypr_dir" HEADLESS-1 > "$OUT/fake_hyprland.log" 2>&1) &
pids="$pids $!"
wait_for "$OUT/fake_hyprland.log" "hazır" 10

(exec env HYPRLAND_INSTANCE_SIGNATURE="$sig" RUST_LOG=warn "$SHELL_BIN" > "$OUT/shell.log" 2>&1) &
shell_pid=$!
pids="$pids $shell_pid"
# Açılıştaki iç bildirimler (ör. kilit ekranı programı yok) süresi dolup çekilsin: kayıt temiz
# bir yığınla başlar.
sleep 8
kill -0 "$shell_pid" || { echo "shell düştü" >&2; exit 1; }
# Kendi örneğimiz mi: kendi örneğimize ping. Değilse hiçbir şey gönderilmez.
"$SHELL_BIN" msg idle status > /dev/null || { echo "test örneğine ping yok" >&2; exit 1; }

(exec wf-recorder -o HEADLESS-1 -r 60 -f "$OUT/rec.mkv" > "$OUT/rec.log" 2>&1) &
rec=$!
send() {
    notify-send -t "$2" -a "Test $1" "Bildirim $1" "Gövde metni $1: kısa bir açıklama satırı." > /dev/null
}
sleep 1.5
# 1) Açık yığın büyür: 1.5 sn arayla 4 bildirim (6 sn süre; ~7.5–12 sn'de birer birer çekilir).
for n in 1 2 3 4; do
    send "$n" 6000
    sleep 1.5
done
# 2) Son kart çekilip yaprak çökerken yeni bildirim: kapanırken geri açılış.
sleep 4.65
send 5 3000
# 3) Yaprak tamamen çöktükten sonra: tohumdan açılış.
sleep 6
send 6 3000
sleep 2
kill -INT "$rec"
wait "$rec" 2> /dev/null || true
rec=""
echo "kayıt: $OUT/rec.mkv"
