#!/bin/sh
# Görünüm kendini onarma uçtan uca (docs/design/look-gtk.md "Kendini onarma"). Gerçek kullanıcı
# düzeni: gtk-4.0/{gtk.css,gtk-dark.css,assets} → root'a ait tema dosyalarına bağ, birleşik görünüm
# açık. Kabuk açılışı → iki CSS Nuxut'un (düz dosya, 0644) → başka bir araç gtk.css'i yeniden
# bağlar → Nuxut onarır ve bir kez bildirir → libadwaita uygulaması (Nautilus) Nuxut renginde.
#
#   <framework>/tools/test-compositor.sh prepare && cargo-gate cargo build
#   flock -o <canlı-test-kilidi> sh tools/live/look-selfheal/run.sh ÇIKTI
#
# Yalıtım: başsız sway, geçici HOME/XDG/dconf, ayrı D-Bus oturumu, kısa geçici XDG_RUNTIME_DIR
# (kabuk varsayılan örnek olarak çalışır: test örneği dosya yazmaz), kabuk ayrı PID ad alanında
# (canlı eylemler kullanıcının süreçlerine ulaşamaz). Çıkış 0 = geçti.
set -eu
OUT=$1
root=$(cd "$(dirname "$0")/../../.." && pwd)
TC=${NUXUT_TEST_COMPOSITOR:-$root/../framework/tools/test-compositor.sh}
export NUXUT_LOOK_SH=${NUXUT_TEST_SHELL:-$root/target/debug/nuxut-shell}
export NUXUT_TEST_NAME=look-selfheal
THEME=${NUXUT_TEST_GTK_THEME:-/usr/share/themes/adw-gtk3-dark/gtk-4.0}
[ -f "$THEME/gtk.css" ] || { echo "tema yok: $THEME (adw-gtk-theme)" >&2; exit 2; }
user_wl=${WAYLAND_DISPLAY:-}
real_rt=${XDG_RUNTIME_DIR:?}
RT=
"$TC" start --owner $$ > /dev/null
trap 'set +e; "$TC" stop; [ -n "$RT" ] && rm -rf "$RT"' EXIT
trap 'exit 130' INT TERM
eval "$("$TC" env)"
[ "$WAYLAND_DISPLAY" != "$user_wl" ] || { echo "display kullanıcınınki" >&2; exit 1; }
unset HYPRLAND_INSTANCE_SIGNATURE DISPLAY XDG_CURRENT_DESKTOP NUXUT_SHELL_INSTANCE
export WAYLAND_DISPLAY="$real_rt/$WAYLAND_DISPLAY"
export GDK_DEBUG=no-portals ADW_DISABLE_PORTAL=1 GIO_USE_VFS=local GVFS_DISABLE_FUSE=1
R=$OUT/root
rm -rf "$R"
mkdir -p "$R/home/empty" "$R/home/.config/nuxut" "$R/home/.config/gtk-4.0" "$R/home/.local/share" "$R/home/.cache" "$R/home/.local/state"
RT=$(mktemp -d /tmp/nxlook.XXXXXX)
chmod 700 "$RT"
printf 'user-db:user\n' > "$R/dconf-profile"
printf '[desktop]\nunified = true\n\n[colors]\nmode = "dark"\ntheme = "sage"\n' > "$R/home/.config/nuxut/framework.toml"
for n in gtk.css gtk-dark.css assets; do ln -s "$THEME/$n" "$R/home/.config/gtk-4.0/$n"; done
export HOME=$R/home XDG_CONFIG_HOME=$R/home/.config XDG_DATA_HOME=$R/home/.local/share \
    XDG_CACHE_HOME=$R/home/.cache XDG_STATE_HOME=$R/home/.local/state DCONF_PROFILE=$R/dconf-profile \
    XDG_RUNTIME_DIR=$RT NUXUT_LOOK_OUT=$OUT THEME
dbus-run-session -- sh "$root/tools/live/look-selfheal/inner.sh"
