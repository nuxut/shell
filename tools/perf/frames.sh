#!/bin/sh
# Kare zamanlaması: framework'ün `nuxut::frame` trace satırlarından (RUST_LOG içinde
# `nuxut::frame=trace`, NO_COLOR=1) gecikme ve kare maliyeti.
#
#   tools/perf/frames.sh burst  LOG BAŞLANGIÇ_NS [BOŞLUK_MS]
#       BAŞLANGIÇ_NS'ten (date +%s%N) sonraki ilk kareye gecikme ve kare patlamasının bitişi
#       (BOŞLUK_MS, varsayılan 400, kadar kare gelmeyince patlama biter).
#       Çıktı: lat_ms settle_ms n p50_ms p95_ms max_ms
#   tools/perf/frames.sh window LOG BAŞLANGIÇ_NS BİTİŞ_NS
#       Aralıktaki tüm kareler. Çıktı: n fps p50_ms p95_ms max_ms
#
# Kare maliyeti = record_us + gl_us + swap_us (kayıt + GL gönderimi + swap/sunum; llvmpipe'ta
# rasterleştirme swap'te). Zaman damgası UTC gün içi saniyeye çevrilir (gece yarısı geçişi yok
# sayılır; ölçümler kısa).
set -eu

# frames LOG: "gün_içi_saniye maliyet_ms" satırları.
frames() {
    awk '/nuxut::frame/ && / kare/ {
        split($1, dt, "T"); split(dt[2], hms, ":")
        sec = hms[1] * 3600 + hms[2] * 60 + substr(hms[3], 1, length(hms[3]) - 1)
        c = 0
        for (i = 2; i <= NF; i++) if ($i ~ /^(record|gl|swap)_us=/) { split($i, kv, "="); c += kv[2] }
        printf "%.6f %.3f\n", sec, c / 1000
    }' "$1"
}

# pct: stdin'den sıralı maliyetler → "p50 p95 max".
pct() {
    sort -n | awk '{ v[NR] = $1 } END {
        if (NR == 0) { print "- - -"; exit }
        i50 = int((NR - 1) * 0.50) + 1; i95 = int((NR - 1) * 0.95) + 1
        printf "%.2f %.2f %.2f\n", v[i50], v[i95], v[NR]
    }'
}

dsec() {
    awk -v ns="$1" 'BEGIN { printf "%.6f", (ns / 1e9) % 86400 }'
}

case ${1:-} in
    burst)
        log=$2
        s=$(dsec "$3")
        gap=${4:-400}
        sel=$(frames "$log" | awk -v s="$s" -v gap="$gap" '
            $1 < s { next }
            first == "" { first = $1; last = $1 }
            ($1 - last) * 1000 > gap { exit }
            { last = $1; print }')
        [ -n "$sel" ] || { echo "- - 0 - - -"; exit 0; }
        hdr=$(echo "$sel" | awk -v s="$s" 'NR == 1 { f = $1 } { l = $1 } END { printf "%.1f %.1f %d", (f - s) * 1000, (l - s) * 1000, NR }')
        echo "$hdr $(echo "$sel" | awk '{ print $2 }' | pct)"
        ;;
    window)
        log=$2
        s=$(dsec "$3")
        e=$(dsec "$4")
        sel=$(frames "$log" | awk -v s="$s" -v e="$e" '$1 >= s && $1 <= e')
        n=$(printf '%s' "$sel" | grep -c . || true)
        fps=$(awk -v n="$n" -v s="$s" -v e="$e" 'BEGIN { printf "%.1f", n / (e - s) }')
        echo "$n $fps $(printf '%s\n' "$sel" | awk 'NF { print $2 }' | pct)"
        ;;
    *)
        sed -n '2,13p' "$0" >&2
        exit 2
        ;;
esac
