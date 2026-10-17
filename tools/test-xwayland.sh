#!/bin/sh
# Yalıtılmış test Xwayland'i: test compositor'ının (framework tools/test-compositor.sh) içinde,
# sabit yüksek bir X display numarasında (:90–:99) rootless Xwayland. X11 istemcileri (xclip,
# xprop…) YALNIZCA `run` ile başlatılır: `run` her çağrıda display'in gerçekten bu betiğin
# başlattığı Xwayland'e ait olduğunu doğrular, değilse komutu hiç çalıştırmaz.
#
# Neden (olay 2026-10-16): `-displayfd` ile başlatılan test Xwayland'i boş display seçip
# dinleyemedi; test xclip'i ~3 sn kullanıcının Xwayland'ine (:1) bağlandı ve kullanıcının
# panosuna test metni yazdı. Kural: testler kullanıcının panosuna/seçimine asla yazmaz, okumaz.
#
#   eval "$(tools/test-compositor.sh env)"     # önce test compositor'ı (WAYLAND_DISPLAY + NUXUT_TEST_STATE)
#   tools/test-xwayland.sh start               # Xwayland'i başlatır, doğrular; `export DISPLAY=:N` yazdırır
#   eval "$(tools/test-xwayland.sh env)"       # DISPLAY (doğrulanamazsa kapalı-güvenli: eval başarısız)
#   tools/test-xwayland.sh run xclip -o …      # doğrula, sonra komutu çalıştır (exec)
#   tools/test-xwayland.sh check               # yalnız doğrula (0 = güvenli)
#   tools/test-xwayland.sh stop                # yalnız kendi Xwayland'ini kapatır
#
# Display numarası: NUXUT_TEST_X_DISPLAY (90–99, varsayılan 93). Numara 90'ın altındaysa,
# kullanıcının DISPLAY'iyle aynıysa ya da /tmp/.X<N>-lock'taki PID bu betiğin başlattığı
# Xwayland değilse hiçbir şey çalıştırılmaz. Test compositor'ı kapanınca Xwayland de çıkar
# (Wayland bağlantısı kopar); `stop` yine de çağrılmalı.
set -eu
unset HYPRLAND_INSTANCE_SIGNATURE

: "${NUXUT_TEST_STATE:?önce: eval \"\$(test-compositor.sh env)\"}"
state=$NUXUT_TEST_STATE
[ -d "$state" ] && [ -O "$state" ] || { echo "test compositor durumu yok: $state" >&2; exit 1; }
# WAYLAND_DISPLAY test compositor'ınki olmalı (kullanıcınınki değil).
[ -s "$state/wayland_display" ] && [ "${WAYLAND_DISPLAY:-}" = "$(cat "$state/wayland_display")" ] || {
    echo "WAYLAND_DISPLAY test compositor'ının değil; çıkılıyor" >&2
    exit 1
}

n=${NUXUT_TEST_X_DISPLAY:-93}
case $n in
    9[0-9]) ;;
    *) echo "NUXUT_TEST_X_DISPLAY 90–99 olmalı: $n" >&2; exit 2 ;;
esac
lock=/tmp/.X$n-lock

proc_start() {
    [ -r "/proc/$1/stat" ] || return 1
    sed 's/^.*) //' "/proc/$1/stat" 2> /dev/null | cut -d' ' -f20
}

proc_running() {
    st=$(sed 's/^.*) //' "/proc/$1/stat" 2> /dev/null | cut -d' ' -f1) || return 1
    [ -n "$st" ] && [ "$st" != Z ] && [ "$st" != X ]
}

# Kayıtlı Xwayland hâlâ aynı süreç mi (pid + başlangıç zamanı); öyleyse pid'i yazdırır.
ours() {
    [ -s "$state/xwayland.pid" ] || return 1
    p=$(cat "$state/xwayland.pid")
    s=$(cat "$state/xwayland.start" 2> /dev/null) || return 1
    [ -n "$s" ] && [ "$(proc_start "$p")" = "$s" ] && proc_running "$p" && echo "$p"
}

# Display gerçekten bizim mi: kayıtlı numara, kilit dosyasındaki PID bizim Xwayland'imiz,
# süreç adı Xwayland, kullanıcının display'i değil.
verify() {
    p=$(ours) || { echo "test Xwayland'i çalışmıyor" >&2; return 1; }
    [ "$(cat "$state/xwayland.display" 2> /dev/null)" = ":$n" ] || { echo "kayıtlı display :$n değil" >&2; return 1; }
    user=$(cat "$state/xwayland.user_display" 2> /dev/null || true)
    [ -z "$user" ] || [ "${user#:}" != "$n" ] || { echo "display kullanıcınınkiyle aynı" >&2; return 1; }
    [ -r "$lock" ] || { echo "$lock yok" >&2; return 1; }
    lp=$(tr -d ' \n' < "$lock")
    [ "$lp" = "$p" ] || { echo "$lock sahibi $lp, bizim Xwayland $p değil" >&2; return 1; }
    [ "$(cat "/proc/$p/comm" 2> /dev/null)" = Xwayland ] || { echo "pid $p Xwayland değil" >&2; return 1; }
    [ -S "/tmp/.X11-unix/X$n" ] || { echo "X$n soketi yok" >&2; return 1; }
}

start() {
    if ours > /dev/null; then
        verify || exit 1
        echo "export DISPLAY=:$n"
        return 0
    fi
    command -v Xwayland > /dev/null || { echo "Xwayland kurulu değil" >&2; exit 1; }
    # Numara dolu (başka bir sürecin kilidi canlı): başkasının display'ine bağlanılmaz.
    if [ -r "$lock" ]; then
        lp=$(tr -d ' \n' < "$lock")
        if [ -n "$lp" ] && proc_running "$lp"; then
            echo ":$n kullanımda (pid $lp); NUXUT_TEST_X_DISPLAY ile başka numara seç" >&2
            exit 1
        fi
    fi
    # Kullanıcının display'i kaydedilir (yalnız karşılaştırma için; ona hiçbir şey bağlanmaz).
    printf '%s\n' "${DISPLAY:-}" > "$state/xwayland.user_display"
    (exec env -u DISPLAY Xwayland ":$n" -rootless -noreset -nolisten tcp > "$state/xwayland.log" 2>&1 < /dev/null) &
    echo $! > "$state/xwayland.pid"
    proc_start $! > "$state/xwayland.start" || : > "$state/xwayland.start"
    echo ":$n" > "$state/xwayland.display"
    i=0
    until verify 2> /dev/null; do
        i=$((i + 1))
        if [ $i -gt 50 ] || ! ours > /dev/null; then
            echo "test Xwayland'i :$n üzerinde doğrulanamadı; günlük: $state/xwayland.log" >&2
            verify || true
            stop > /dev/null 2>&1 || true
            exit 1
        fi
        sleep 0.1
    done
    echo "export DISPLAY=:$n"
}

stop() {
    p=$(ours) || { rm -f "$state/xwayland.pid" "$state/xwayland.start"; return 0; }
    kill -TERM "$p" 2> /dev/null || true
    i=0
    while proc_running "$p" && [ $i -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
    proc_running "$p" && kill -KILL "$p" 2> /dev/null
    sleep 0.1
    if proc_running "$p"; then
        echo "Xwayland $p kapanmadı" >&2
        return 1
    fi
    rm -f "$state/xwayland.pid" "$state/xwayland.start"
}

cmd=${1:-}
[ $# -gt 0 ] && shift
case $cmd in
    start) start ;;
    env)
        if verify; then
            echo "export DISPLAY=:$n"
        else
            # Kapalı-güvenli: var olmayan display + eval başarısız.
            echo "export DISPLAY=:nuxut-test-xwayland-yok"
            echo "false"
        fi
        ;;
    check) verify ;;
    run)
        [ $# -gt 0 ] || { echo "run KOMUT…" >&2; exit 2; }
        verify || { echo "X11 istemcisi başlatılmadı" >&2; exit 1; }
        DISPLAY=:$n
        export DISPLAY
        exec "$@"
        ;;
    stop) stop ;;
    *) sed -n '2,24p' "$0" >&2; exit 2 ;;
esac
