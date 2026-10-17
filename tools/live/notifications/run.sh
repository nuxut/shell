#!/bin/sh
# Bildirim popup kaydı (docs/design/panels.md §4.1): YALNIZCA izole headless sway'de, geçici
# HOME/XDG ve özel D-Bus oturumunda. Kullanıcının oturumuna, Nuxut süreçlerine ve bus'larına
# hiçbir şey gönderilmez.
#
#   cargo-gate cargo build --bin nuxut-shell --example fake_hyprland   # kilit DIŞINDA
#   ../framework/tools/test-compositor.sh prepare                      # kilit DIŞINDA
#   flock -o ../.live-test.lock sh tools/live/notifications/run.sh [ÇIKTI_DİZİNİ]
#   sh tools/live/notifications/frames.sh ÇIKTI_DİZİNİ                 # kare dökümü + ölçüm
#
# Senaryo: shell açılır, kayıt başlar, 1.5 sn arayla 4 bildirim gelir (yığın her seferinde
# büyür). Sonuç: ÇIKTI_DİZİNİ/rec.mkv (60 kare/sn, HEADLESS-1) ve süreç logları.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
TC=${NUXUT_TC:-$root/../framework/tools/test-compositor.sh}
OUT=${1:-$XDG_RUNTIME_DIR/nuxut-test/notifications}
mkdir -p "$OUT"
rm -f "$OUT"/rec.mkv "$OUT"/*.log

user_display=${WAYLAND_DISPLAY:-}
"$TC" start --owner $$ > /dev/null
trap 'set +e; "$TC" stop > /dev/null' EXIT
trap 'exit 130' INT TERM
eval "$("$TC" env)"
if [ -z "${WAYLAND_DISPLAY:-}" ] || [ "$WAYLAND_DISPLAY" = "$user_display" ]; then
    echo "WAYLAND_DISPLAY test compositor'ınki değil; çıkılıyor" >&2
    exit 2
fi
unset HYPRLAND_INSTANCE_SIGNATURE DISPLAY

# Geçici HOME ve ayar dizinleri: kullanıcının config'i, dconf'u ve dosyaları okunmaz/yazılmaz.
tmp=$(mktemp -d "$XDG_RUNTIME_DIR/nuxut-test-notif.XXXXXX")
trap 'set +e; rm -rf "$tmp"; "$TC" stop > /dev/null' EXIT
export HOME=$tmp/home XDG_CONFIG_HOME=$tmp/config XDG_DATA_HOME=$tmp/data XDG_STATE_HOME=$tmp/state XDG_CACHE_HOME=$tmp/cache
mkdir -p "$HOME" "$XDG_CONFIG_HOME/nuxut" "$XDG_DATA_HOME" "$XDG_STATE_HOME" "$XDG_CACHE_HOME"
export DCONF_PROFILE=/dev/null GDK_DEBUG=no-portals NO_COLOR=1 GIO_USE_VFS=local

dbus-run-session -- sh "$here/scenario.sh" "$OUT" "$root"
