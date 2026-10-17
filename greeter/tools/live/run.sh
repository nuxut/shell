#!/bin/sh
# nuxut-greeter canlı testi: YALNIZCA izole headless sway içinde, sahte greetd soketiyle (gerçek
# kimlik doğrulama yok). Kullanıcının oturumuna, greetd'sine ve /etc'ye dokunmaz.
#
#   ../framework/tools/test-compositor.sh prepare                          # kilit dışında
#   cargo-gate cargo build -p nuxut-greeter --examples --bins             # kilit dışında
#   flock -o <canlı test kilidi> sh greeter/tools/live/run.sh
#
# Akış: iki çıkış → yanlış şifre (hata satırı) → Aşağı ile ikinci kullanıcı (PAM "hesap kilitli"
# mesajı) → Yukarı → doğru şifre → sahte greetd start_session alır, greeter 0 ile çıkar, son
# seçim state.toml'a yazılır. Görüntüler $OUT'ta (yalnızca test compositor'ının çıkışları).
set -eu

root=$(cd "$(dirname "$0")/../../.." && pwd)
TC=${NUXUT_TC:-$root/../framework/tools/test-compositor.sh}
export NUXUT_TEST_NAME=${NUXUT_TEST_NAME:-greeter-test}
BIN=$root/target/debug/nuxut-greeter
FAKE=$root/target/debug/examples/fake_greetd
OUT=${NUXUT_GREETER_LIVE_OUT:-$XDG_RUNTIME_DIR/nuxut-test/greeter-live}
mkdir -p "$OUT"
[ -x "$BIN" ] && [ -x "$FAKE" ] || { echo "ikililer yok (önce derle)" >&2; exit 2; }

user_display=${WAYLAND_DISPLAY:-}
"$TC" start --owner $$ > /dev/null
pids=""
cleanup() {
    set +e
    for p in $pids; do kill "$p" 2> /dev/null; done
    sleep 0.5
    for p in $pids; do kill -0 "$p" 2> /dev/null && kill -9 "$p" 2> /dev/null; done
    rm -f "${sock:-}"
    "$TC" stop > /dev/null 2>&1
}
trap cleanup EXIT INT TERM HUP
eval "$("$TC" env)"
if [ -z "${WAYLAND_DISPLAY:-}" ] || [ "$WAYLAND_DISPLAY" = "$user_display" ]; then
    echo "WAYLAND_DISPLAY test compositor'ınki değil; çıkılıyor" >&2
    exit 2
fi
unset HYPRLAND_INSTANCE_SIGNATURE DISPLAY
swaymsg create_output > /dev/null

fail() { echo "BAŞARISIZ: $*" >&2; exit 1; }
wait_for() { # wait_for DOSYA DESEN SANİYE
    i=0
    while [ "$i" -lt $(($3 * 10)) ]; do
        grep -q -- "$2" "$1" 2> /dev/null && return 0
        sleep 0.1
        i=$((i + 1))
    done
    return 1
}

tmp=$(mktemp -d "$OUT/run.XXXXXX")
# Soket yolu kısa olmalı (SUN_LEN).
sock=$(mktemp -u "$XDG_RUNTIME_DIR/nuxut-greetd.XXXXXX")
mkdir -p "$tmp/sessions" "$tmp/state" "$tmp/config" "$tmp/icons"
cat > "$tmp/passwd" << 'EOF'
root:x:0:0::/root:/bin/sh
ayse:x:1000:1000:Ayşe Yılmaz,,,:/home/ayse:/bin/sh
kilitli:x:1001:1001:Deneme Hesabı:/home/kilitli:/bin/sh
EOF
printf '[Desktop Entry]\nName=Nuxut\nExec=/usr/lib/nuxut/nuxut-session\nDesktopNames=Hyprland\n' > "$tmp/sessions/nuxut.desktop"
printf '[Desktop Entry]\nName=Sway\nExec=sway\nDesktopNames=sway\n' > "$tmp/sessions/sway.desktop"

(exec "$FAKE" "$sock" > "$OUT/fake.log" 2>&1) &
fake_pid=$!
pids="$fake_pid"
wait_for "$OUT/fake.log" dinleniyor 10 || fail "sahte greetd başlamadı"

export NUXUT_GREETER_INSTANCE=greeter-test
(
    export GREETD_SOCK="$sock" NUXUT_GREETER_PASSWD="$tmp/passwd" \
        NUXUT_GREETER_SESSIONS="$tmp/sessions" NUXUT_GREETER_ICONS="$tmp/icons" \
        NUXUT_GREETER_STATE_DIR="$tmp/state" XDG_CONFIG_HOME="$tmp/config" \
        NUXUT_GREETER_KB_LAYOUTS=tr,us NUXUT_GREETER_POWER=poweroff,reboot
    exec "$BIN" > "$OUT/greeter.log" 2>&1
) &
greeter_pid=$!
pids="$pids $greeter_pid"
wait_for "$OUT/greeter.log" "giriş ekranı" 20 || fail "greeter başlamadı"

status() { "$BIN" msg status 2> /dev/null || true; }
wait_status() { # wait_status EVRE SANİYE
    i=0
    while [ "$i" -lt $(($2 * 10)) ]; do
        [ "$(status)" = "$1" ] && return 0
        sleep 0.1
        i=$((i + 1))
    done
    echo "evre: $(status)" >&2
    return 1
}
# Form klavye odağındaki çıkışta: ikisi de çekilir.
shot() { sleep 0.6; grim -o HEADLESS-1 "$OUT/$1-a.png"; grim -o HEADLESS-2 "$OUT/$1-b.png"; }

wait_status prompt 15 || fail "şifre istemi gelmedi"
shot 01-prompt

# Klavye düzeni: Tab ile oturum → düzen düğmesi, Enter → TR'den US'e (Hyprland yok: yalnız etiket).
wtype -k Tab
wtype -k Tab
wtype -k Return
grep -q 'klavye düzeni: Hyprland yok' "$OUT/greeter.log" 2>/dev/null || sleep 0.5
shot 01b-layout
wtype -M shift -k Tab -m shift
wtype -M shift -k Tab -m shift

wtype yanlis
wtype -k Return
wait_for "$OUT/fake.log" 'Denied("ayse")' 10 || fail "yanlış şifre gönderilmedi"
wait_status prompt 10 || fail "yanlış şifreden sonra istem yok"
shot 02-wrong

wtype -k Down
wait_for "$OUT/fake.log" 'Create("kilitli")' 10 || fail "kullanıcı değişmedi"
wait_status prompt 10 || fail "ikinci kullanıcı istemi yok"
wtype x
wtype -k Return
wait_for "$OUT/fake.log" 'Denied("kilitli")' 10 || fail "kilitli hesap reddedilmedi"
wait_status prompt 10 || true
shot 03-locked-account

wtype -k Up
wait_for "$OUT/fake.log" 'Cancel' 10 || fail "iptal yok"
sleep 0.5
wait_status prompt 10 || fail "ilk kullanıcıya dönülmedi"
wtype nuxut
wtype -k Return
wait_for "$OUT/fake.log" 'Start' 10 || fail "start_session gelmedi"
i=0
while kill -0 "$greeter_pid" 2> /dev/null && [ $i -lt 30 ]; do sleep 0.1; i=$((i + 1)); done
kill -0 "$greeter_pid" 2> /dev/null && fail "greeter oturum başlatılınca çıkmadı"
wait "$greeter_pid" || fail "greeter çıkış kodu sıfır değil"
grep -q 'user = "ayse"' "$tmp/state/state.toml" || fail "son seçim yazılmadı"
grep -q 'session = "nuxut"' "$tmp/state/state.toml" || fail "oturum yazılmadı"
grep -F 'Start' "$OUT/fake.log"
grep -qF '"/usr/lib/nuxut/nuxut-session"' "$OUT/fake.log" || fail "Nuxut oturumu başlatılmadı"
echo "GEÇTİ"
