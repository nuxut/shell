# İkincil örnek fail-open düzeltmesi (instance.rs), deterministik: test kancası birincili
# bekletir (yalnızca fake-auth + NUXUT_LOCK_INSTANCE; NUXUT_LOCK_TEST_HOLD_*_MS).
#  A) birincil `pending`'de: ikincil hemen 0 ile çıkmaz; birincil kilitlenince "locked" görür,
#     ready yazar, 0 ile çıkar.
#  B) birincil `done`'da (açıldı, çıkışı bekletiliyor): ikincil "kilitli" saymaz; birincil
#     çıkınca kilidi KENDİSİ kurar, ready'yi kendi kilidinden sonra yazar; şifreyle açılır.
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"

# spawn LOG READY [ENV=DEĞER…] — kendi örneğimizi --ready-fd ile başlatır; spawned_pid.
spawn() {
    log=$1 ready=$2
    shift 2
    rm -f "$log" "$ready"
    (
        export NUXUT_LOCK_INSTANCE=test-lock NUXUT_LOCK_FAKE_SECRET="$SECRET" NO_COLOR=1 \
            RUST_LOG="$LOCK_RUST_LOG"
        for kv in "$@"; do export "$kv"; done
        exec "$BIN" --ready-fd 3 3> "$ready" > "$log" 2>&1
    ) &
    spawned_pid=$!
    pids="$pids $spawned_pid"
}

# exit_of PID SANİYE — süreç biterse çıkış kodu $code; bitmezse başarısız (alt kabukta
# çağrılmaz: wait yalnızca ana kabuğun çocuğunu bekler).
exit_of() {
    i=0
    while kill -0 "$1" 2> /dev/null; do
        [ "$i" -lt $(($2 * 10)) ] || fail "süreç $1 $2 sn içinde çıkmadı"
        sleep 0.1
        i=$((i + 1))
    done
    wait "$1" && code=0 || code=$?
}

# --- A: pending ---
spawn "$OUT/08a-primary.log" "$OUT/08a-primary.ready" NUXUT_LOCK_TEST_HOLD_PENDING_MS=2000
primary=$spawned_pid
wait_for "$OUT/08a-primary.log" "test kancası: kilit isteği bekletiliyor" 5 ||
    fail "A: kanca etkin değil (log: $OUT/08a-primary.log)"
spawn "$OUT/08a-second.log" "$OUT/08a-second.ready"
second=$spawned_pid
sleep 1
kill -0 "$second" 2> /dev/null || fail "A: ikincil, birincil pending iken çıktı (fail-open)"
[ -s "$OUT/08a-second.ready" ] && fail "A: ikincil kilitten önce ready yazdı"
exit_of "$second" 10
[ "$code" -eq 0 ] || fail "A: ikincil çıkış kodu $code"
grep -q "oturum kilitlendi" "$OUT/08a-primary.log" || fail "A: birincil kilitlenmedi"
grep -q "zaten bir kilit örneği çalışıyor (kilitli)" "$OUT/08a-second.log" ||
    fail "A: ikincil kilidi görmedi"
[ "$(cat "$OUT/08a-second.ready")" = locked ] || fail "A: ikincil ready yazmadı"
lock_log=$OUT/08a-primary.log lock_pid=$primary
wait_first_frame
unlock
echo "A (pending): geçti"

# --- B: done ---
spawn "$OUT/08b-primary.log" "$OUT/08b-primary.ready" NUXUT_LOCK_TEST_HOLD_DONE_MS=2000
primary=$spawned_pid
wait_for "$OUT/08b-primary.ready" locked 10 || fail "B: birincil kilitlenmedi"
lock_log=$OUT/08b-primary.log
wait_first_frame
"$TC" key Escape
"$TC" type "$SECRET"
"$TC" key Return
wait_for "$lock_log" "test kancası: çıkış bekletiliyor" 5 || fail "B: birincil done'a geçmedi"
spawn "$OUT/08b-second.log" "$OUT/08b-second.ready"
second=$spawned_pid
sleep 0.5
grep -q "(kilitli)" "$OUT/08b-second.log" && fail "B: ikincil done'ı kilitli saydı (fail-open)"
[ -s "$OUT/08b-second.ready" ] && fail "B: ikincil kendi kilidinden önce ready yazdı"
exit_of "$primary" 5
[ "$code" -eq 0 ] || fail "B: birincil çıkış kodu $code"
wait_for "$OUT/08b-second.ready" locked 10 || fail "B: ikincil kilidi kendisi kurmadı"
grep -q "oturum kilitlendi" "$OUT/08b-second.log" || fail "B: ikincil locked logu yok"
lock_log=$OUT/08b-second.log lock_pid=$second
wait_first_frame
unlock
echo "B (done): geçti"
echo "08-secondary: geçti"
