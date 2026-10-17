# Temel akış: kilitle → yanlış şifre (açılmaz, süreç yaşar) → doğru şifre → çıkış 0.
# Ekran görüntüsü: kilitliyken HEADLESS-1 (opak, içerik yalnızca kilit yüzeyi).
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"

start_lock 01-unlock
"$TC" shot "$OUT/01-locked.png"

"$TC" type yanlis-sifre
"$TC" key Return
wait_for "$lock_log" "doğrulama bitti" 5 || fail "yanlış şifre doğrulaması bitmedi"
sleep 0.5
kill -0 "$lock_pid" 2> /dev/null || fail "yanlış şifreyle süreç çıktı"
grep -q "kilit bitti" "$lock_log" && fail "yanlış şifreyle kilit bitti"

# Artan gecikme: ilk başarısızlıktan sonra 2 sn beklenir (state.rs).
sleep 2.5
unlock
grep -q "oturum kilitlendi" "$lock_log" || fail "locked logu yok"
echo "01-unlock: geçti"
