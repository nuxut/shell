# Ekran okuyucu (ADR-0011 madde 8): AT yalnız okur. YALNIZ yalıtılmış a11y ortamında:
#
#   cargo-gate cargo build -p nuxut-lock --features fake-auth && cargo-gate cargo build --example at_probe
#   flock -o <canlı-test-kilidi> framework/tools/a11y-e2e.sh sh lock/tools/live/09-a11y.sh
#
# (a11y-e2e.sh başsız sway'i, ayrı oturum bus'ını ve a11y bus'ını kurar; burada HOME de geçici.)
# Denetlenen: düşmanca istemci (examples/at_probe) parola alanına yazamaz (EditableText yok), Text
# ile içeriği okuyamaz, hiçbir düğümde eylem yok; boş parolada gerçek Enter PAM denemesi
# başlatmaz; gerçek Enter doğru şifreyle açar. Sahte parola "abc123": değeri ve uzunluğu ağaçta,
# günlükte (debug dahil) ve Orca çıktısında yok, text-changed olayı yok; yanlış şifre genel metinle
# okunur. ORCA=1 ise gerçek Orca (ses çıkmaz).
here=$(cd "$(dirname "$0")" && pwd)
[ "${NUXUT_A11Y_E2E:-}" = 1 ] || { echo "yalnız framework/tools/a11y-e2e.sh içinde" >&2; exit 2; }
# a11y-e2e WAYLAND_DISPLAY'ı test compositor'ınkine zaten kurdu; common.sh yeniden doğrular.
unset WAYLAND_DISPLAY
. "$here/common.sh"
PROBE=${NUXUT_AT_PROBE:-$root/target/debug/examples/at_probe}
[ -x "$PROBE" ] || { echo "at_probe yok: $PROBE" >&2; exit 2; }
export HOME="$OUT/home" LANG=tr_TR.UTF-8
mkdir -p "$HOME"
busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true

if [ "${ORCA:-0}" = 1 ]; then
    # Konuşma sunucusuna ulaşmayan adres: kullanıcının hoparlöründen ses çıkmaz.
    SPEECHD_ADDRESS=unix_socket:$OUT/yok.sock orca --debug-file="$OUT/09-orca.log" > "$OUT/09-orca.out" 2>&1 &
    orca_pid=$!
    pids="$pids $orca_pid"
    sleep 4
fi

# Tüm modüller debug: a11y modüllerinin RUST_LOG ne olursa olsun sessiz kaldığı denetlenir.
LOCK_RUST_LOG=debug
start_lock 09-a11y

# Boş parola: gerçek Enter PAM denemesi başlatmaz.
"$TC" key Return
sleep 1
grep -q "doğrulama başladı" "$lock_log" && fail "boş parolada doğrulama başladı"

# Sahte parola: ne değeri ne uzunluğu ağaçta, olaylarda, Orca çıktısında ya da günlükte.
"$TC" type abc123
sleep 0.5
"$PROBE" --wait 10 --inspect Şifre --action Şifre --set-text Şifre --action "Kilit ekranı" > "$OUT/09-probe.txt" 2>&1 || true
cat "$OUT/09-probe.txt"
grep -q "^inspect Şifre: rol " "$OUT/09-probe.txt" || fail "parola düğümü bulunamadı"
grep "^inspect Şifre:" "$OUT/09-probe.txt" | grep -q "EditableText" && fail "parola alanında EditableText var"
grep "^inspect Şifre:" "$OUT/09-probe.txt" | grep -q "metin hata" || fail "parola Text ile okunabiliyor"
grep "^action " "$OUT/09-probe.txt" | grep -q "DoAction true" && fail "AT eylemi uygulandı"
grep -q "YAZILDI" "$OUT/09-probe.txt" && fail "AT parolaya yazdı"
grep -q "doğrulama başladı" "$lock_log" && fail "AT doğrulama başlattı"
kill -0 "$lock_pid" 2> /dev/null || fail "kilit AT ile kapandı"

# Yanlış şifre: durum canlı bölgeden okunur (genel metin).
"$TC" key Return
wait_for "$lock_log" "doğrulama bitti" 5 || fail "yanlış şifre doğrulaması bitmedi"
sleep 1.5

# Günlük (RUST_LOG=debug dahil): sır, maske ve AT olayı yok.
grep -q -e abc123 -e "$(printf '\342\200\242')" "$lock_log" && fail "sır ya da maske loga yazıldı"
grep -q -i -e "erişilebilirlik" -e "AT eylemi" -e accesskit "$lock_log" && fail "AT olayı loga yazıldı"

if [ "${ORCA:-0}" = 1 ]; then
    speech=$OUT/09-speech.txt
    grep -a "SPEECH OUTPUT" "$OUT/09-orca.log" > "$speech" || true
    head -n 20 "$speech"
    grep -q -e abc123 -e "$(printf '\342\200\242')" -e "6 karakter" -e "altı" "$speech" && fail "Orca sırrı ya da uzunluğu okudu"
    # text-changed olayı yok (tuş başına uzunluk/karakter duyurusu olmaz).
    grep -a -q "Dequeued object:text-changed" "$OUT/09-orca.log" && fail "text-changed olayı gönderildi"
    # Açıklama yalnız boş↔dolu geçişinde değişir (6 tuşta bir kez; gönderimde boşalır): en çok 2.
    n=$(grep -a -c "Dequeued object:property-change:accessible-description" "$OUT/09-orca.log" || true)
    [ "$n" -le 2 ] || fail "parola alanı tuş başına olay gönderiyor ($n)"
    grep -q "Yanlış şifre" "$speech" || fail "Orca yanlış şifreyi okumadı"
fi

# Artan gecikme (2 sn) sonrası gerçek Enter doğru şifreyle açar.
sleep 1
unlock
echo "09-a11y: geçti"
