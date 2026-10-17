# G7 (askıya alma öncesi kısım): uyku inhibitor'ı tutan çağıran `--ready-fd`'yi bekler ve
# yalnızca kilit gerçekten kurulunca ("locked") inhibitor'ı bırakır. Burada doğrulanan:
#  - "locked\n" yalnızca compositor `locked`'ından ve tüm kilit yüzeylerinin ilk karesinden SONRA;
#  - ikinci örnek (zaten kilitli) kilit kurmadan hemen "locked" yazar ve 0 ile çıkar;
#  - geçersiz fd kilidi engellemez.
# Gerçek suspend/resume (logind PrepareForSleep, DPMS sonrası ilk kare) bu compositor'da
# yapılamaz: ELLE, kullanıcının onayıyla ve shell tetikleyicisi açıldıktan sonra denenecek.
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"

start_lock 06-ready
grep -q "oturum kilitlendi" "$lock_log" || fail "ready yazıldı ama locked logu yok"
# ready yalnızca tüm kilit yüzeylerinin ilk karesinden sonra (log satırı on_locked'tan önce).
grep -q "kilit kuruldu: ilk kareler sunuldu" "$lock_log" || fail "ready ilk kareden önce yazıldı"
[ "$(cat "$lock_ready")" = locked ] || fail "ready içeriği 'locked' değil"

# İkinci örnek: aynı ad, tek örnek IPC'si üzerinden durum sorar.
second_ready=$OUT/06-second.ready
rm -f "$second_ready"
(
    export NUXUT_LOCK_INSTANCE=test-lock NUXUT_LOCK_FAKE_SECRET="$SECRET" NO_COLOR=1 RUST_LOG="$LOCK_RUST_LOG"
    exec "$BIN" --ready-fd 3 3> "$second_ready" > "$OUT/06-second.log" 2>&1
) &
second_pid=$!
pids="$pids $second_pid"
wait "$second_pid" && code=0 || code=$?
[ "$code" -eq 0 ] || fail "ikinci örnek çıkış kodu $code"
grep -q "zaten bir kilit örneği" "$OUT/06-second.log" || fail "ikinci örnek tek örneği görmedi"
[ "$(cat "$second_ready")" = locked ] || fail "ikinci örnek ready yazmadı"

unlock

# Geçersiz fd (açık olmayan 9): uyarı, kilit yine kurulur.
lock_log=$OUT/06-badfd.log
(
    export NUXUT_LOCK_INSTANCE=test-lock NUXUT_LOCK_FAKE_SECRET="$SECRET" NO_COLOR=1 RUST_LOG="$LOCK_RUST_LOG"
    exec "$BIN" --ready-fd 9 > "$lock_log" 2>&1
) &
lock_pid=$!
pids="$pids $lock_pid"
wait_for "$lock_log" "oturum kilitlendi" 10 || fail "geçersiz fd'de kilit kurulmadı"
grep -q "ready-fd kullanılamıyor" "$lock_log" || fail "geçersiz fd uyarısı yok"
wait_first_frame
unlock
echo "06-ready-fd: geçti (gerçek suspend/resume elle bekliyor)"
