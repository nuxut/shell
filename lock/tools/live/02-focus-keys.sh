# Odak değişirken yazılan tuşlar kilit dışına gitmez (framework 952e960 keymap düzeltmesi).
# Kendi kitty --class nuxut-test penceremiz odaktayken kilit başlatılır ve kilit kurulurken
# sürekli yazılır; hiçbir bayt pencereye ulaşmamalı (kilitten önce yazılanlar hariç: işaret
# baytından sonra hiçbir şey olmamalı).
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"

start_capture 02-focus
# Kilitten önce yazılan işaret pencereye ulaşmalı: tuzak gerçekten çalışıyor.
"$TC" type "ONCE|"
wait_for "$capture" "ONCE|" 5 || fail "tuzak penceresi tuş almıyor; test geçersiz"

# Kilidi başlat ve aynı anda yaz (kilit kurulma penceresi).
(
    i=0
    while [ "$i" -lt 40 ]; do
        "$TC" type "x" || true
        i=$((i + 1))
    done
) &
typer=$!
pids="$pids $typer"
start_lock 02-focus
wait "$typer" || true
# Kilitliyken de yaz.
"$TC" type "KILITLI"
sleep 0.5

after=$(sed -n 's/^ONCE|//p' "$capture" | tr -d 'x')
[ -z "$after" ] || fail "kilitliyken pencereye tuş ulaştı: $(wc -c < "$capture") bayt"
# Kilit kurulmadan önce yazılan x'ler pencereye gidebilir (meşru); ama 'locked'tan sonra
# hiçbiri: ready dosyası yazıldığı andaki bayt sayısı sonra artmamalı.
n1=$(wc -c < "$capture")
"$TC" type "xxxx"
sleep 0.5
n2=$(wc -c < "$capture")
[ "$n1" -eq "$n2" ] || fail "locked sonrası pencere $((n2 - n1)) bayt aldı"

# Açılmadan önce yazılan şifre de pencereye gitmemeli.
unlock
sleep 0.3
grep -q "$SECRET" "$capture" && fail "şifre pencereye ulaştı"
echo "02-focus-keys: geçti"
