# G9: kilit süreci kill -9 ile ölünce oturum kilitli kalır (ext-session-lock: compositor açmaz).
# Kanıt: ölümden sonra yazılan tuşlar tuzak penceresine ulaşmaz; ikinci bir kilit örneği
# kilidi devralıp normal açar.
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"

start_capture 03-kill9
start_lock 03-first
kill -9 "$lock_pid"
wait "$lock_pid" 2> /dev/null || true
sleep 0.5
"$TC" shot "$OUT/03-after-kill.png"

n1=$(wc -c < "$capture")
"$TC" type "OLUMDEN-SONRA"
sleep 0.5
n2=$(wc -c < "$capture")
[ "$n1" -eq "$n2" ] || fail "kill -9 sonrası oturum açık: pencere $((n2 - n1)) bayt aldı"

# Kurtarma: yeni örnek kilidi devralır (sway terk edilmiş kilide yeni istemci kabul eder).
start_lock 03-second
unlock
echo "03-kill9: geçti (görüntü: $OUT/03-after-kill.png — opak olmalı, pencere görünmemeli)"
