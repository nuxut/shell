#!/bin/sh
# Boşta maliyet ölçümü: bir sürecin CPU'su, uyanmaları (bağlam değişimleri) ve belleği.
# Yalnızca /proc okur; sürece hiçbir şey göndermez. Kendi test örneğin dışında kullanma.
#
#   tools/perf/probe.sh idle PID SANİYE      # SANİYE boyunca bekler, özet + thread dökümü
#   tools/perf/probe.sh mem PID              # RSS/PSS/tepe RSS (MB)
#   tools/perf/probe.sh maps PID             # PSS’e en çok katkı veren eşlemeler
#
# Çıktı (idle): ilk satır `cpu=%.2f% wake=%.1f/s rss=MB pss=MB hwm=MB threads=N`; ardından
# o aralıkta uyanan her thread: `  tid ad uyanma(gönüllü+zorunlu) cpu_ms`.
# Uyanma = gönüllü bağlam değişimi (thread bloklandı → sonra uyandı) + zorunlu (kesildi).
set -eu

hz=$(getconf CLK_TCK)

# snap PID: her thread için "tid ad utime+stime(tick) gönüllü zorunlu" satırı.
snap() {
    for t in /proc/"$1"/task/*; do
        tid=${t##*/}
        # comm boşluk/parantez içerebilir: stat'ı son ')'ten sonra böl.
        st=$(cat "$t/stat" 2> /dev/null) || continue
        name=$(cat "$t/comm" 2> /dev/null | tr ' ' '_') || continue
        rest=${st##*) }
        set -- $rest
        # rest: 1=durum ... 12=utime 13=stime
        ticks=$((${12} + ${13}))
        v=$(awk '/^voluntary_ctxt_switches/ {print $2}' "$t/status" 2> /dev/null || echo 0)
        n=$(awk '/^nonvoluntary_ctxt_switches/ {print $2}' "$t/status" 2> /dev/null || echo 0)
        echo "$tid $name $ticks $v $n"
    done
}

mem() {
    # Dökülemez (PR_SET_DUMPABLE=0: kilit ekranı) süreçte smaps okunamaz: RSS status'tan, PSS yok.
    if [ -r "/proc/$1/smaps_rollup" ] && awk '/^Rss:/ {r=$2} /^Pss:/ {p=$2} END {printf "rss=%.1f pss=%.1f", r/1024, p/1024}' "/proc/$1/smaps_rollup" 2> /dev/null; then
        :
    else
        awk '/^VmRSS:/ {printf "rss=%.1f pss=?", $2/1024}' "/proc/$1/status"
    fi
    awk '/^VmHWM:/ {printf " hwm=%.1f", $2/1024} /^Threads:/ {printf " threads=%d", $2}' "/proc/$1/status"
    echo
}

case ${1:-} in
    idle)
        pid=$2
        secs=$3
        [ -d "/proc/$pid" ] || { echo "süreç yok: $pid" >&2; exit 1; }
        a=$(mktemp)
        b=$(mktemp)
        trap 'rm -f "$a" "$b"' EXIT
        snap "$pid" > "$a"
        t0=$(date +%s%N)
        sleep "$secs"
        snap "$pid" > "$b"
        t1=$(date +%s%N)
        m=$(mem "$pid")
        # Ölen thread'in sayaçları kaybolur (yalnız yaşayanlar ve yeni doğanlar sayılır).
        awk -v hz="$hz" -v ns=$((t1 - t0)) -v m="$m" '
            NR == FNR { cpu[$1] = $3; w[$1] = $4 + $5; next }
            {
                dc = $3 - (($1 in cpu) ? cpu[$1] : 0)
                dw = $4 + $5 - (($1 in w) ? w[$1] : 0)
                tc += dc; tw += dw
                if (dw > 0 || dc > 0) line[++k] = sprintf("  %s %s %d %d", $1, $2, dw, dc * 1000 / hz)
            }
            END {
                s = ns / 1e9
                printf "cpu=%.2f%% wake=%.1f/s %s\n", tc / hz / s * 100, tw / s, m
                for (i = 1; i <= k; i++) print line[i]
            }' "$a" "$b"
        ;;
    mem)
        mem "$2"
        ;;
    maps)
        # PSS'e en çok katkı veren 12 eşleme (kütüphane adı ya da [heap]/[anon]), MB.
        awk '/^[0-9a-f]+-[0-9a-f]+ / { name = (NF >= 6) ? $6 : "[anon]"; sub(/.*\//, "", name) }
             /^Pss:/ { pss[name] += $2 }
             /^Anonymous:/ { anon[name] += $2 }
             END { for (n in pss) if (pss[n] >= 512) printf "%8.1f %8.1f %s\n", pss[n] / 1024, anon[n] / 1024, n }' \
            "/proc/$2/smaps" | sort -rn | head -12 | sed 's/^/  pss_mb anon_mb: /'
        ;;
    *)
        sed -n '2,10p' "$0" >&2
        exit 2
        ;;
esac
