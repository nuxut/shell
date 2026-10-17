# nuxut-lock canlı test ortak parçaları (kaynak olarak okunur: `. "$here/common.sh"`).
#
# Kilit etkinleştirme kontrol listesi (docs/ARCHITECTURE.md "Tetikleme") için betikler.
# YALNIZCA izole headless sway'de çalışır; kullanıcının Hyprland oturumu asla kilitlenmez.
#
#   cargo-gate cargo build -p nuxut-lock --features fake-auth      # debug + sahte doğrulayıcı
#   flock -o "$XDG_RUNTIME_DIR/nuxut-live-test.lock" lock/tools/live/run.sh <betik> [<betik>…]
#
# run.sh her betik için compositor'ı `start --owner $$` ile başlatır (sahibi ölünce sway kapanır)
# ve trap'inde `set +e` ile durdurur. Betikler compositor'ı kendileri başlatmaz; çalışmıyorsa
# `env` kapalı-güvenli ortam verir ve betik 2 ile durur.
#
# Her betik: 0 = geçti, 1 = başarısız (neden stderr'de), 2 = ortam hazır değil.
# Loglar ve görüntüler $NUXUT_LOCK_LIVE_OUT'a (varsayılan: $XDG_RUNTIME_DIR/nuxut-test/test-lock-live).
# Görüntüler yalnızca test compositor'ının çıkışlarıdır (kullanıcı ekranı değil); iş bitince silinir.

set -eu

: "${NUXUT_TEST_NAME:=test-lock}"
export NUXUT_TEST_NAME
root=$(cd "$(dirname "$0")/../../.." && pwd)
TC=${NUXUT_TC:-$root/../framework/tools/test-compositor.sh}
BIN=${NUXUT_LOCK_BIN:-$root/target/debug/nuxut-lock}
# Sahte doğrulayıcının şifresi (yalnızca fake-auth derlemesi + adlandırılmış örnek kabul eder).
SECRET=nuxut-live-1
OUT=${NUXUT_LOCK_LIVE_OUT:-$XDG_RUNTIME_DIR/nuxut-test/test-lock-live}
mkdir -p "$OUT"

# Test compositor'ı; kapalıysa env kapalı-güvenli bir WAYLAND_DISPLAY verir ve betik durur.
user_display=${WAYLAND_DISPLAY:-}
eval "$("$TC" env)"
if [ -z "${WAYLAND_DISPLAY:-}" ] || [ "$WAYLAND_DISPLAY" = "$user_display" ]; then
    echo "WAYLAND_DISPLAY test compositor'ınki değil; çıkılıyor" >&2
    exit 2
fi
unset HYPRLAND_INSTANCE_SIGNATURE DISPLAY

[ -x "$BIN" ] || { echo "ikili yok: $BIN" >&2; exit 2; }
grep -a -q NUXUT_LOCK_FAKE_SECRET "$BIN" || {
    echo "ikili fake-auth özelliğiyle derlenmemiş: $BIN" >&2
    exit 2
}

# Başlatılan her süreç buraya; betik nasıl biterse bitsin öldürülür (yalnızca kendi PID'lerimiz).
pids=""
cleanup() {
    set +e
    for p in $pids; do
        kill "$p" 2> /dev/null || true
    done
    sleep 0.5
    for p in $pids; do
        if kill -0 "$p" 2> /dev/null; then
            kill -9 "$p" 2> /dev/null || true
        fi
    done
}
trap cleanup EXIT INT TERM HUP

fail() {
    echo "BAŞARISIZ: $*" >&2
    exit 1
}

# wait_for DOSYA DESEN SANİYE [SATIR] — dosyada (SATIR'dan sonra) desen görünene dek bekler.
wait_for() {
    i=0
    from=$((${4:-0} + 1))
    while [ "$i" -lt $(($3 * 10)) ]; do
        if [ -f "$1" ] && tail -n "+$from" "$1" | grep -q -- "$2"; then
            return 0
        fi
        sleep 0.1
        i=$((i + 1))
    done
    return 1
}

# start_lock AD — kendi örneğimizi başlatır; lock_pid, lock_log, lock_ready ayarlanır.
# Örnek adı her zaman tanımlı: IPC kullanıcının kilidine gitmez, sahte doğrulayıcı açılır.
start_lock() {
    lock_log=$OUT/$1.log
    lock_ready=$OUT/$1.ready
    rm -f "$lock_log" "$lock_ready"
    (
        # NO_COLOR: log alanları düz metin (grep "surfaces=2").
        export NUXUT_LOCK_INSTANCE=test-lock NUXUT_LOCK_FAKE_SECRET="$SECRET" NO_COLOR=1 \
            RUST_LOG="$LOCK_RUST_LOG"
        exec "$BIN" --ready-fd 3 3> "$lock_ready" > "$lock_log" 2>&1
    ) &
    lock_pid=$!
    pids="$pids $lock_pid"
    wait_for "$lock_ready" locked 10 || fail "$1: 10 sn içinde kilitlenmedi (log: $lock_log)"
    grep -q "SAHTE doğrulayıcı" "$lock_log" || fail "$1: sahte doğrulayıcı açılmadı"
    wait_first_frame
}

# İlk kare (framework açılış profili) ve kilidin değiştirici/odak logları debug; geri kalan info.
LOCK_RUST_LOG=${LOCK_RUST_LOG:-info,nuxut_lock=debug,nuxut_wayland::profile=debug}

# wait_first_frame — kilit yüzeyinin ilk karesi çizilene dek bekler. sway klavye odağını kilit
# yüzeyine ilk karesinden sonra verir (llvmpipe'ta 'locked'tan ~140 ms sonra); o ana dek
# gönderilen sanal tuşlar hiçbir yere gitmez (sızmaz, ama şifrenin baştan bir kısmı kaybolur).
# Odak olayının kendisi işaret olamaz: başsız sway'de gerçek klavye yok, wl_keyboard ancak
# wtype'ın geçici sanal klavyesiyle (ilk tuşta) gelir.
wait_first_frame() {
    wait_for "$lock_log" "ilk kare" 10 || fail "kilit yüzeyi ilk karesini çizmedi (log: $lock_log)"
    sleep 0.1
}

# unlock — alanı Escape ile temizler (kilitliyken yazılmış artıklar), şifreyi yazıp Return;
# süreç 0 ile çıkmalı.
unlock() {
    "$TC" key Escape
    "$TC" type "$SECRET"
    "$TC" key Return
    i=0
    while kill -0 "$lock_pid" 2> /dev/null; do
        [ "$i" -lt 100 ] || fail "açılmadı (log: $lock_log)"
        sleep 0.1
        i=$((i + 1))
    done
    wait "$lock_pid" && code=0 || code=$?
    [ "$code" -eq 0 ] || fail "nuxut-lock çıkış kodu $code (log: $lock_log)"
}

# start_capture AD — girdi tuzağı: kendi kitty penceremiz, tuşları ham kipte dosyaya yazar.
start_capture() {
    capture=$OUT/$1.keys
    : > "$capture"
    kitty --class nuxut-test -o confirm_os_window_close=0 \
        sh "$root/lock/tools/live/capture.sh" "$capture" > "$OUT/$1.kitty.log" 2>&1 &
    kitty_pid=$!
    pids="$pids $kitty_pid"
    i=0
    until swaymsg -t get_tree | jq -e '.. | objects | select(.app_id? == "nuxut-test" and .focused?)' > /dev/null; do
        [ "$i" -lt 100 ] || fail "nuxut-test penceresi odak almadı"
        sleep 0.1
        i=$((i + 1))
    done
}
