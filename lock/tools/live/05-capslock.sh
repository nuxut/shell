# F2: Caps Lock uyarısı. `keylock caps on`'dan HEMEN sonra (wtype'tan önce) uyarı görünmeli:
# her wtype kendi sanal klavyesiyle modifier'ları sıfırlar, araya yazma girerse test anlamsız.
# Kanıt: debug logunda caps_lock=true değiştirici olayı + ekran görüntüsü (uyarı satırı).
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"

caps_off() { "$TC" keylock caps off || true; }
trap 'cleanup; caps_off' EXIT INT TERM HUP

start_lock 05-caps
n=$(wc -l < "$lock_log")
"$TC" keylock caps on
wait_for "$lock_log" "caps_lock=true" 3 "$n" || fail "Caps Lock değiştiricisi kilide ulaşmadı"
sleep 0.3
"$TC" shot "$OUT/05-caps-on.png"

n=$(wc -l < "$lock_log")
"$TC" keylock caps off
wait_for "$lock_log" "caps_lock=false" 3 "$n" || fail "Caps Lock kapanışı kilide ulaşmadı"
sleep 0.3
"$TC" shot "$OUT/05-caps-off.png"

unlock
echo "05-capslock: geçti (görüntüler: $OUT/05-caps-*.png — yalnızca 'on'da uyarı satırı)"
