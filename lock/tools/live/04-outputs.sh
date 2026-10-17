# G5: iki çıkış ve kilitliyken çıkış ekle/çıkar. Her çıkışta bir kilit yüzeyi; yeni çıkış
# kilitliyken hemen kaplanır, çıkarılan çıkışın yüzeyi düşer, kilit açık kalmaz.
# Ek çıkışlar YALNIZCA test compositor'ında (kullanıcının Hyprland'ında çıkış eklemek yasak).
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"

second=$("$TC" output add 1920x1080@1)
third=""
remove_outputs() {
    [ -n "$third" ] && "$TC" output remove "$third" || true
    [ -n "$second" ] && "$TC" output remove "$second" || true
}
trap 'cleanup; remove_outputs' EXIT INT TERM HUP

start_lock 04-outputs
wait_for "$lock_log" "surfaces=2" 5 || fail "iki çıkışta iki kilit yüzeyi yok"
"$TC" shot "$OUT/04-a.png" HEADLESS-1
"$TC" shot "$OUT/04-b.png" "$second"

# Kilitliyken üçüncü çıkış (ölçekli): kaplanmalı.
third=$("$TC" output add 2560x1440@1.5)
wait_for "$lock_log" "surfaces=3" 5 || fail "kilitliyken eklenen çıkış kaplanmadı"
"$TC" shot "$OUT/04-c.png" "$third"

# Kilitliyken çıkar: süreç yaşamalı, kilit sürmeli.
"$TC" output remove "$second"
second=""
sleep 0.5
kill -0 "$lock_pid" 2> /dev/null || fail "çıkış çıkarılınca kilit süreci öldü"
grep -q "kilit bitti" "$lock_log" && fail "çıkış çıkarılınca kilit bitti"

unlock
echo "04-outputs: geçti (görüntüler: $OUT/04-*.png — hepsi opak kilit yüzeyi olmalı)"
