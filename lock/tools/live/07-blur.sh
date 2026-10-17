# F4: duvar kağıdı bulanıklığı. Üretilmiş dama tahtası duvar kağıdıyla (kullanıcı görseli değil)
# blur=false ve blur=true kilitlenir; içerikten uzak köşe kırpılıp kenar yoğunluğu ölçülür:
# bulanıkta keskinin yarısından az olmalı. Config ve palet ortamdan yalıtılır (XDG_*_HOME),
# kullanıcının ~/.config/nuxut/lock.toml'u okunmaz.
here=$(cd "$(dirname "$0")" && pwd)
. "$here/common.sh"

cfg=$OUT/07-config
state=$OUT/07-state
mkdir -p "$cfg/nuxut" "$state"
wall=$OUT/07-wall.png
magick -size 1920x1080 xc:black -fx "(floor(i/64)+floor(j/64))%2" "$wall"
export XDG_CONFIG_HOME="$cfg" XDG_STATE_HOME="$state"

# edge DOSYA — sol üst 320x180 köşede (saat/alan dışında) ortalama kenar şiddeti (0..1).
edge() {
    magick "$1" -crop 320x180+0+0 +repage -colorspace Gray -edge 1 "$1.edge.png"
    magick identify -format "%[fx:mean]" "$1.edge.png"
}

for mode in false true; do
    printf 'wallpaper = "%s"\nblur = %s\ndim = 0.0\n' "$wall" "$mode" > "$cfg/nuxut/lock.toml"
    start_lock "07-blur-$mode"
    # Bulanık kopya doku hazır olunca (bir sonraki kare) çizilir; geçiş hareketine pay.
    sleep 1.5
    "$TC" shot "$OUT/07-$mode.png"
    unlock
done

sharp=$(edge "$OUT/07-false.png")
blurred=$(edge "$OUT/07-true.png")
echo "kenar: keskin=$sharp bulanık=$blurred"
awk -v s="$sharp" -v b="$blurred" 'BEGIN { exit !(s > 0.01 && b < s / 2) }' \
    || fail "bulanıklık görünmüyor (keskin=$sharp bulanık=$blurred)"
echo "07-blur: geçti (görüntüler: $OUT/07-*.png)"
