#!/bin/sh
# xdg-desktop-portal-hyprland (xdph) ayarına Nuxut ekran paylaşım seçicisini işaretli blokla
# ekler ya da kaldırır. install.sh çağırır; tek başına da çalışır. Tasarım:
# docs/design/screenshare-picker.md.
#
#   xdph-picker.sh install SEÇİCİ-YOLU   blok yoksa ekler, varsa yolunu günceller (idempotent)
#   xdph-picker.sh uninstall             bloğu kaldırır (dosyada başka bir şey yoksa dosyayı da)
#   xdph-picker.sh plan SEÇİCİ-YOLU      hiçbir şey yazmadan ne yapılacağını söyler
#
# Dosya: ${XDG_CONFIG_HOME:-$HOME/.config}/hypr/xdph.conf (xdph'in okuduğu yol). Kurallar:
# - Kullanıcının dosyasına yazmadan önce yedek: <dosya>.nuxut-yedek-<zaman> (yalnızca içerik
#   değişiyorsa; değişiklik yoksa hiçbir şey yazılmaz).
# - Yalnızca kendi bloğumuza dokunulur; dosyanın geri kalanı bayt bayt korunur (sondaki boş
#   satırlar hariç).
# - Blok dışında custom_picker_binary varsa (kullanıcının kendi seçicisi) dosyaya dokunulmaz.
# - Dosya sembolik bağsa (dotfile yöneticisi) ya da işaretler bozuksa dokunulmaz; elle eklenecek
#   satırlar yazdırılır.
# Çıkış: 0 tamam (ya da zaten güncel), 3 bilerek atlandı (neden stdout'ta), 1 hata, 2 kullanım.
# xdph ayarını yalnızca açılışta okur: etkisi bir sonraki girişte (ya da xdph yeniden başlayınca).
set -eu

CONF=${XDG_CONFIG_HOME:-${HOME:?HOME gerekli}/.config}
FILE=$CONF/hypr/xdph.conf
BEGIN_PREFIX='# >>> nuxut-sharepicker'
END_PREFIX='# <<< nuxut-sharepicker'
BEGIN="$BEGIN_PREFIX: install.sh yönetir (kaldırmak için: sh install.sh --uninstall ya da bu bloğu silin) >>>"
END="$END_PREFIX <<<"

usage() {
    echo "kullanım: xdph-picker.sh install|plan SEÇİCİ-YOLU | uninstall" >&2
    exit 2
}

block() { # block YOL
    printf '%s\nscreencopy {\n    custom_picker_binary = %s\n}\n%s\n' "$BEGIN" "$1" "$END"
}

manual() { # manual YOL — elle eklenecek satırlar
    echo "elle eklemek için $FILE dosyasına:"
    echo "    screencopy {"
    echo "        custom_picker_binary = $1"
    echo "    }"
}

# Bizim bloğumuz dışındaki içerik, sondaki boş satırlar atılmış.
outside_block() {
    [ -f "$FILE" ] || return 0
    awk -v b="$BEGIN_PREFIX" -v e="$END_PREFIX" '
        index($0, b) == 1 { skip = 1; next }
        index($0, e) == 1 { skip = 0; next }
        !skip
    ' "$FILE" | sed -e :a -e '/^[[:space:]]*$/{$d;N;ba' -e '}'
}

# İşaretler tutarlı mı: ya hiç yok ya da tam bir çift (önce başlangıç).
markers_ok() {
    [ -f "$FILE" ] || return 0
    awk -v b="$BEGIN_PREFIX" -v e="$END_PREFIX" '
        index($0, b) == 1 { nb++; if (open) bad = 1; open = 1; next }
        index($0, e) == 1 { ne++; if (!open) bad = 1; open = 0; next }
        END { exit (bad || open || nb > 1 || nb != ne) ? 1 : 0 }
    ' "$FILE"
}

has_block() {
    [ -f "$FILE" ] && grep -q "^$BEGIN_PREFIX" "$FILE"
}

# Blok dışında etkin (yorum olmayan) custom_picker_binary var mı.
user_picker() {
    outside_block | grep -Eq '^[^#]*custom_picker_binary'
}

safe_path() {
    case $1 in /*) ;; *) return 1 ;; esac
    # hyprlang değeri: '#' yorum başlatır, boşluk/denetim karakteri satırı bozar.
    case $1 in *[!A-Za-z0-9/._+-]*) return 1 ;; esac
}

# Yazmadan önce denetimler; atlanacaksa nedeni yazıp 3 döner.
precheck() { # precheck YOL|""
    if [ -L "$FILE" ]; then
        echo "atlandı: $FILE sembolik bağ (dotfile yöneticisi?); dokunulmadı."
        [ -n "$1" ] && manual "$1"
        return 3
    fi
    if [ -e "$FILE" ] && [ ! -f "$FILE" ]; then
        echo "atlandı: $FILE düzenli dosya değil; dokunulmadı."
        return 3
    fi
    if ! markers_ok; then
        echo "atlandı: $FILE içindeki nuxut-sharepicker işaretleri bozuk; dokunulmadı (bloğu elle düzeltin ya da silin)."
        return 3
    fi
    return 0
}

# new_content DOSYA YOL|"" — yeni içerik geçici dosyaya.
new_content() {
    {
        rest=$(outside_block)
        if [ -n "$rest" ]; then
            printf '%s\n' "$rest"
            if [ -n "$2" ]; then printf '\n'; fi
        fi
        if [ -n "$2" ]; then block "$2"; fi
    } > "$1"
}

# commit GEÇİCİ — içerik değiştiyse yedekle ve atomik yer değiştir; boşsa dosyayı kaldır.
commit() {
    if [ -f "$FILE" ] && cmp -s "$1" "$FILE"; then
        rm -f "$1"
        echo "değişiklik yok: $FILE güncel."
        return 0
    fi
    if [ -f "$FILE" ]; then
        backup="$FILE.nuxut-yedek-$(date +%Y%m%d-%H%M%S)"
        cp -p "$FILE" "$backup"
        echo "yedek: $backup"
    fi
    if [ ! -s "$1" ]; then
        rm -f "$1" "$FILE"
        echo "kaldırıldı: $FILE (Nuxut bloğundan başka içerik yoktu)."
        return 0
    fi
    mv -f "$1" "$FILE"
    echo "yazıldı: $FILE"
}

cmd=${1:-}
case $cmd in
    install|plan)
        picker=${2:-}
        [ -n "$picker" ] || usage
        if ! safe_path "$picker"; then
            echo "atlandı: seçici yolu güvenli değil ($picker): mutlak ve yalnızca [A-Za-z0-9/._+-] olmalı."
            exit 3
        fi
        rc=0; precheck "$picker" || rc=$?
        [ "$rc" = 0 ] || exit "$rc"
        if user_picker; then
            echo "atlandı: $FILE içinde başka bir custom_picker_binary var (sizin seçiciniz); dokunulmadı."
            exit 3
        fi
        if [ "$cmd" = plan ]; then
            if has_block; then
                echo "[deneme] $FILE: Nuxut bloğu güncellenirdi (custom_picker_binary = $picker)."
            elif [ -f "$FILE" ]; then
                echo "[deneme] $FILE: sona Nuxut bloğu eklenirdi (önce yedek)."
            else
                echo "[deneme] $FILE oluşturulurdu (yalnızca Nuxut bloğu)."
            fi
            exit 0
        fi
        mkdir -p "$(dirname "$FILE")"
        tmp="$FILE.nuxut-yeni.$$"
        trap 'rm -f "$tmp"' EXIT
        new_content "$tmp" "$picker"
        commit "$tmp"
        ;;
    uninstall)
        if ! has_block; then
            echo "değişiklik yok: $FILE içinde Nuxut bloğu yok."
            exit 0
        fi
        rc=0; precheck "" || rc=$?
        [ "$rc" = 0 ] || exit "$rc"
        tmp="$FILE.nuxut-yeni.$$"
        trap 'rm -f "$tmp"' EXIT
        new_content "$tmp" ""
        commit "$tmp"
        ;;
    *) usage ;;
esac
