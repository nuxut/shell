#!/bin/sh
# run.sh'in ayrı D-Bus oturumu içindeki kısmı; ortam run.sh'te kurulur.
set -eu
OUT=${NUXUT_LOOK_OUT:?}
SH=${NUXUT_LOOK_SH:?}
G=$HOME/.config/gtk-4.0
[ "$HOME" = "$OUT/root/home" ] || exit 1
spid=; npid=
owner_pid() { busctl --user call org.freedesktop.DBus /org/freedesktop/DBus org.freedesktop.DBus GetConnectionUnixProcessID s org.gnome.Nautilus 2>/dev/null | awk '{print $2}'; }
cleanup() {
    set +e
    p=$(owner_pid); [ -n "$p" ] && kill "$p"
    [ -n "$npid" ] && kill "$npid"
    [ -n "$spid" ] && kill "$spid"
    sleep 1
}
trap cleanup EXIT
fail=0
expect_ours() {
    for n in gtk.css gtk-dark.css; do
        if [ -L "$G/$n" ] || [ ! -f "$G/$n" ]; then echo "HATA ($1): $n Nuxut'un değil"; fail=1; continue; fi
        m=$(stat -c %a "$G/$n")
        [ "$m" = 644 ] || { echo "HATA ($1): $n izni $m"; fail=1; }
        grep -q 'nuxut.css' "$G/$n" || { echo "HATA ($1): $n nuxut.css içermiyor"; fail=1; }
    done
}
wait_ours() { i=0; while [ $i -lt 40 ]; do if [ -f "$G/gtk.css" ] && [ ! -L "$G/gtk.css" ] && [ ! -L "$G/gtk-dark.css" ]; then return 0; fi; sleep 0.5; i=$((i+1)); done; return 0; }
(exec unshare --user --map-current-user --pid --fork --mount --mount-proc "$SH" > "$OUT/shell.log" 2>&1) &
spid=$!
wait_ours; sleep 1
expect_ours "açılış"
echo "== başka bir araç gtk.css'i yeniden bağlıyor"
rm -f "$G/gtk.css"; ln -s "$THEME/gtk.css" "$G/gtk.css"
sleep 2; wait_ours; sleep 1
expect_ours "onarma"
grim "$OUT/repaired.png"
echo "== Nautilus"
(exec nautilus --new-window "$HOME/empty" > "$OUT/nautilus.log" 2>&1) &
npid=$!
sleep 5
rect=$(swaymsg -t get_tree | jq -r '[.. | objects | select(.app_id? == "org.gnome.Nautilus")][0].rect | "\(.x),\(.y) \(.width)x\(.height)"')
grim -g "$rect" "$OUT/nautilus.png"
want=$(grep -o -- '--view-bg-color: #[0-9a-f]*' "$G/nuxut.css" | head -1 | awk '{print toupper($2)}')
got=$(magick "$OUT/nautilus.png" -crop "1x1+900+540" -depth 8 txt:- | tail -1 | awk '{print $3}')
echo "içerik rengi: $got (Nuxut view-bg: $want)"
[ "$got" = "$want" ] || { echo "HATA: Nautilus Nuxut renginde değil"; fail=1; }
[ "$fail" = 0 ] && echo "GEÇTİ" || echo "KALDI"
exit "$fail"
