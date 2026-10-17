#!/bin/sh
# run.sh'ın iç adımı: özel dbus-run-session bus'ında. Doğrudan çalıştırma.
#   scenario.sh ÇIKTI_DİZİNİ KÖK
set -eu

OUT=$1
root=$2
here=$root/tools/perf
probe="sh $here/probe.sh"
frames="sh $here/frames.sh"
R=$OUT/results.txt
IDLE=${NUXUT_PERF_IDLE:-60}
ONLY=${NUXUT_PERF_ONLY:-shell lock settings}
SHELL_BIN=${NUXUT_PERF_SHELL:-$root/target/release/nuxut-shell}
LOCK_BIN=${NUXUT_PERF_LOCK:-$root/target/release/nuxut-lock}
SETTINGS_BIN=${NUXUT_PERF_SETTINGS:-$root/../settings/target/debug/nuxut-settings}
EX=${NUXUT_PERF_EXAMPLES:-$root/target/debug/examples}
case ${DBUS_SESSION_BUS_ADDRESS:-} in
    unix:path=/tmp/* | unix:abstract=* | unix:path=$XDG_RUNTIME_DIR/*) ;;
    *) echo "özel oturum bus'ı yok" >&2; exit 2 ;;
esac
# Sahte NM/UPower/BlueZ özel oturum bus'ında; shell sistem bus'ı olarak onu görür.
export DBUS_SYSTEM_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS
export NUXUT_SHELL_INSTANCE=test-perf NUXUT_SETTINGS_INSTANCE=test-perf NUXUT_LOCK_INSTANCE=test-perf

pids=""
cleanup() {
    set +e
    for p in $pids; do kill "$p" 2> /dev/null; done
    sleep 0.5
    for p in $pids; do kill -0 "$p" 2> /dev/null && kill -9 "$p" 2> /dev/null; done
    [ -n "${hypr_dir:-}" ] && rm -rf "$hypr_dir"
}
trap cleanup EXIT INT TERM HUP

say() { echo "$*" >> "$R"; }

# wait_for DOSYA DESEN SANİYE
wait_for() {
    i=0
    while [ "$i" -lt $(($3 * 20)) ]; do
        grep -q -- "$2" "$1" 2> /dev/null && return 0
        sleep 0.05
        i=$((i + 1))
    done
    return 1
}

# start_app AD KOMUT… → app_pid, app_log; başlangıç anı t_start (ns).
start_app() {
    app_log=$OUT/$1.log
    # Önceki koşunun logu ilk kare beklemesini yanıltmasın.
    : > "$app_log"
    shift
    t_start=$(date +%s%N)
    (exec "$@" > "$app_log" 2>&1) &
    app_pid=$!
    pids="$pids $app_pid"
}

stop_app() {
    kill "$app_pid" 2> /dev/null || true
    i=0
    while kill -0 "$app_pid" 2> /dev/null && [ $i -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
    kill -0 "$app_pid" 2> /dev/null && kill -9 "$app_pid" 2> /dev/null
    wait "$app_pid" 2> /dev/null || true
}

# first_frame AD: "ilk kare" profil satırı + duvar saati (exec → log satırı görülene dek).
first_frame() {
    wait_for "$app_log" "ilk kare" 30 || { say "$1: ilk kare 30 sn içinde yok"; return 1; }
    t=$(date +%s%N)
    say "$1 açılış: duvar≈$(( (t - t_start) / 1000000 )) ms | $(grep -m1 'ilk kare' "$app_log" | sed 's/^.*profile[^ ]* //')"
}

# idle AD: IDLE sn boşta ölçüm + o aralıktaki kareler.
idle() {
    sleep 5
    a=$(date +%s%N)
    res=$($probe idle "$app_pid" "$IDLE")
    b=$(date +%s%N)
    say "$1 boşta ${IDLE} sn: $(echo "$res" | head -1) | kareler(n fps p50 p95 max): $($frames window "$app_log" "$a" "$b")"
    echo "$res" | tail -n +2 | sed "s/^/$1 thread /" >> "$R"
}

MSG() { "$SHELL_BIN" msg "$@" > /dev/null; }

# latency AD KOMUT…: komuttan ilk kareye ve animasyon bitişine.
latency() {
    name=$1
    shift
    t=$(date +%s%N)
    MSG "$@"
    sleep 1.2
    say "  $name: $($frames burst "$app_log" "$t")"
}

if echo "$ONLY" | grep -q shell; then
    # Sahte Hyprland (kullanıcınınkinden farklı imza), sahte NM/UPower/BlueZ, sahte MPRIS.
    sig=nuxut-perf-$$
    hypr_dir=$XDG_RUNTIME_DIR/hypr/$sig
    (exec "$EX/fake_hyprland" "$hypr_dir" HEADLESS-1 > "$OUT/fake_hyprland.log" 2>&1) &
    pids="$pids $!"
    (exec "$EX/fake_status" --wifi 70 --battery 64 --bt on > "$OUT/fake_status.log" 2>&1) &
    pids="$pids $!"
    (exec "$EX/fake_mpris" --title "Parça" --artist "Sanatçı" > "$OUT/fake_mpris.log" 2>&1) &
    pids="$pids $!"
    wait_for "$OUT/fake_hyprland.log" "hazır" 10
    sleep 1

    for run in soğuk ılık; do
        start_app shell env HYPRLAND_INSTANCE_SIGNATURE="$sig" NUXUT_PROFILE_STARTUP=1 \
            RUST_LOG="warn,nuxut::frame=trace,nuxut_wayland=info" "$SHELL_BIN"
        first_frame "shell($run)"
        # Kendi örneğimiz mi: log + kendi örneğimize ping. Değilse komut gönderilmez.
        MSG idle status || { say "shell örneğine ping yok"; exit 1; }
        [ "$run" = ılık ] || stop_app
    done
    say "shell renderer: $(grep -m1 'GL context hazır' "$app_log" | sed 's/^.*hazır //')"
    idle shell
    $probe mem "$app_pid" | sed 's/^/shell bellek (boşta): /' >> "$R"
    $probe maps "$app_pid" >> "$R"

    # Hızlı kip: yalnızca açılış + boşta (karşılaştırmalı deneyler için).
    [ -z "${NUXUT_PERF_QUICK:-}" ] || exit 0
    say "shell gecikme (lat_ms settle_ms n p50 p95 max):"
    for i in 1 2 3; do
        latency "sağ panel aç#$i" panel open right
        latency "sağ panel kapat#$i" panel close right
        latency "saat paneli aç#$i" panel open clock
        latency "saat paneli kapat#$i" panel close clock
        latency "başlatıcı aç#$i" launcher open
        latency "başlatıcı kapat#$i" launcher close
    done
    $probe mem "$app_pid" | sed 's/^/shell bellek (paneller sonrası): /' >> "$R"

    # Olay fırtınası: MPRIS parça değişimi 40/sn, bildirim 10/sn, workspace geçişi 20/sn; 10 sn.
    sleep 2
    storm_end=$(($(date +%s) + 10))
    (
        while [ "$(date +%s)" -lt "$storm_end" ]; do
            dbus-send --session --dest=org.mpris.MediaPlayer2.nuxuttest /org/mpris/MediaPlayer2 \
                org.mpris.MediaPlayer2.Player.Next > /dev/null 2>&1 || true
            sleep 0.025
        done
    ) &
    s1=$!
    (
        n=0
        while [ "$(date +%s)" -lt "$storm_end" ]; do
            n=$((n + 1))
            notify-send -a perf "Bildirim $n" "Gövde metni $n" > /dev/null 2>&1 || true
            sleep 0.1
        done
    ) &
    s2=$!
    (
        n=0
        while [ "$(date +%s)" -lt "$storm_end" ]; do
            n=$((n % 5 + 1))
            printf 'dispatch workspace %s' "$n" | socat - "UNIX-CONNECT:$hypr_dir/.socket.sock" > /dev/null 2>&1 || true
            sleep 0.05
        done
    ) &
    s3=$!
    pids="$pids $s1 $s2 $s3"
    a=$(date +%s%N)
    res=$($probe idle "$app_pid" 10)
    b=$(date +%s%N)
    wait $s1 $s2 $s3 2> /dev/null || true
    say "shell fırtına 10 sn: $(echo "$res" | head -1) | kareler(n fps p50 p95 max): $($frames window "$app_log" "$a" "$b")"
    echo "$res" | tail -n +2 | sed 's/^/shell fırtına thread /' >> "$R"
    sleep 8
    $probe mem "$app_pid" | sed 's/^/shell bellek (fırtına sonrası): /' >> "$R"
    stop_app
fi

if echo "$ONLY" | grep -q settings && [ -x "$SETTINGS_BIN" ]; then
    start_app settings env NUXUT_PROFILE_STARTUP=1 RUST_LOG="warn,nuxut::frame=trace,nuxut_wayland=info" "$SETTINGS_BIN"
    first_frame settings || true
    idle settings
    stop_app
fi

if echo "$ONLY" | grep -q lock; then
    start_app lock env NUXUT_PROFILE_STARTUP=1 RUST_LOG="warn,nuxut::frame=trace,nuxut_wayland=info" "$LOCK_BIN"
    first_frame lock || true
    idle lock
    stop_app
fi
