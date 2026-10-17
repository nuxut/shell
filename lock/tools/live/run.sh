#!/bin/bash
# Canlı betikleri çalıştırır: her betik için taze izole başsız sway (start --owner $$ → betik →
# stop). Sahip bu süreç: betik nasıl biterse bitsin (kill -9 dahil) compositor kapanır.
#
#   flock -o "$XDG_RUNTIME_DIR/nuxut-live-test.lock" lock/tools/live/run.sh 01-unlock 02-focus-keys …
#
# Çıkış: hepsi geçtiyse 0, aksi hâlde 1. Betik çıktıları $NUXUT_LOCK_LIVE_OUT/<betik>.run.
set -eu
export NUXUT_TEST_NAME=${NUXUT_TEST_NAME:-test-lock}
here=$(cd "$(dirname "$0")" && pwd)
TC=${NUXUT_TC:-$here/../../../../framework/tools/test-compositor.sh}
export NUXUT_LOCK_LIVE_OUT=${NUXUT_LOCK_LIVE_OUT:-$XDG_RUNTIME_DIR/nuxut-test/test-lock-live}
mkdir -p "$NUXUT_LOCK_LIVE_OUT"
trap 'set +e; "$TC" stop > /dev/null 2>&1' EXIT
failed=0
for t in "$@"; do
    "$TC" start --owner $$ > /dev/null
    set +e
    timeout 120 sh "$here/$t.sh" > "$NUXUT_LOCK_LIVE_OUT/$t.run" 2>&1
    rc=$?
    set -e
    echo "== $t rc=$rc"
    tail -n 8 "$NUXUT_LOCK_LIVE_OUT/$t.run"
    [ "$rc" -eq 0 ] || failed=1
    "$TC" stop > /dev/null || { echo "compositor durdurulamadı" >&2; exit 1; }
done
exit "$failed"
