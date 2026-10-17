#!/bin/sh
# Performans ölçüm turu (docs/research/perf-2026-10.md): YALNIZCA izole headless sway'de.
# Kullanıcının oturumuna, Nuxut süreçlerine ve bus'larına hiçbir şey gönderilmez.
#
#   cargo-gate cargo build --release -p nuxut-shell -p nuxut-lock     # ölçülen ikililer
#   cargo-gate cargo build --examples                                  # fake_* (debug)
#   flock -o "$XDG_RUNTIME_DIR/nuxut-live-test.lock" sh tools/perf/run.sh [ÇIKTI_DİZİNİ]
#
# Ortam: NUXUT_PERF_SHELL / _LOCK / _SETTINGS (ikili yolları), NUXUT_PERF_IDLE (sn, 60),
# NUXUT_PERF_ONLY (shell|lock|settings boşlukla ayrılmış; varsayılan hepsi), NUXUT_PERF_QUICK=1
# (yalnız açılış + boşta), NUXUT_PERF_SHELL_TOML (test örneğinin shell.toml'u). LP_NUM_THREADS
# gibi ortam değişkenleri ölçülen süreçlere geçer.
# Sonuç: ÇIKTI_DİZİNİ/results.txt (+ her sürecin logu).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
TC=${NUXUT_TC:-$root/../framework/tools/test-compositor.sh}
OUT=${1:-$XDG_RUNTIME_DIR/nuxut-test/test-perf}
mkdir -p "$OUT"
export NUXUT_TEST_NAME=test-perf

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

# Yalıtılmış ayar dizinleri: kullanıcının config'i ve dconf'u okunmaz/yazılmaz.
cfg=$(mktemp -d "$XDG_RUNTIME_DIR/nuxut-test/test-perf-cfg.XXXXXX")
trap 'set +e; rm -rf "$cfg"; "$TC" stop > /dev/null' EXIT
export XDG_CONFIG_HOME=$cfg/config XDG_DATA_HOME=$cfg/data XDG_STATE_HOME=$cfg/state XDG_CACHE_HOME=$cfg/cache
mkdir -p "$XDG_CONFIG_HOME/nuxut" "$XDG_DATA_HOME" "$XDG_STATE_HOME" "$XDG_CACHE_HOME"
# İsteğe bağlı shell.toml (ör. tek widget'ın maliyetini ayırmak için); yoksa varsayılanlar.
[ -z "${NUXUT_PERF_SHELL_TOML:-}" ] || cp "$NUXUT_PERF_SHELL_TOML" "$XDG_CONFIG_HOME/nuxut/shell.toml"
export DCONF_PROFILE=/dev/null GDK_DEBUG=no-portals NO_COLOR=1

# Yük durumu: ölçüm yorumunda gerekir (eşzamanlı derlemeler).
{
    echo "# $(date -Is) yük: $(cut -d' ' -f1-3 /proc/loadavg)"
    echo "# derleyen süreçler: $(ps -eo comm= | grep -cE '^(rustc|cc1|ld|cargo)$' || true)"
} > "$OUT/results.txt"

dbus-run-session -- sh "$here/scenario.sh" "$OUT" "$root"
cat "$OUT/results.txt"
