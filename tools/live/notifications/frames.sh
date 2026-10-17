#!/bin/sh
# run.sh kaydının kare dökümü (kilit gerekmez; ekrana çizmez):
#   frames.sh ÇIKTI_DİZİNİ [KIRPMA]   KIRPMA ffmpeg crop biçiminde, varsayılan 400:480:1520:30
#                                     (1920x1080 HEADLESS-1, üst bar, popup sütunu)
# Çıktı: ÇIKTI_DİZİNİ/frames/NNNN.png (30 kare/sn) ve her karenin gri standart sapması (x1000)
# satır satır. Boş koyu kutu karesi sapmanın çökmesiyle görünür; şüpheli kareleri gözle incele
# (`magick montage` ile yan yana).
set -eu
OUT=$1
CROP=${2:-400:480:1520:30}
rm -rf "$OUT/frames"
mkdir -p "$OUT/frames"
ffmpeg -v error -i "$OUT/rec.mkv" -vf "fps=30,crop=$CROP" "$OUT/frames/%04d.png"
magick identify -format "%t %[fx:int(standard_deviation*1000)]\n" "$OUT"/frames/*.png
