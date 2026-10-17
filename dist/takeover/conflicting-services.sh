#!/bin/sh
# Nuxut'la çakışan bar ve bildirim servislerini devralır ya da geri getirir. install.sh çağırır;
# tek başına da çalışır. Tasarım: docs/design/service-takeover.md.
#
#   conflicting-services.sh apply [--now]  devral: kaydet, devre dışı bırak; --now ile çalışanları
#                                          da durdur (yalnız Nuxut kabuğu çalışacaksa; idempotent)
#   conflicting-services.sh restore   kayıttaki her şeyi eski hâline getir, kaydı sil
#   conflicting-services.sh plan      hiçbir şey yapmadan ne yapılacağını söyler
#
# Kapsam: dunst, mako, swaync (org.freedesktop.Notifications adını tutar) ve waybar, quickshell
# (qs), ags, eww (ikinci bar/kabuk). Kurallar:
# - Önce kayıt: değişen her şey $STATE/nuxut/takeover/services'e YAZILDIKTAN SONRA değiştirilir
#   (birimin önceki etkinlik durumu, çalışıyor muydu, maskeledik mi; durdurulan süreçler).
#   İlk kayıt kazanır: ikinci apply özgün durumu ezmez.
# - systemd kullanıcı birimi: disable --now. Bildirim servisleri ayrıca maskelenir: D-Bus
#   etkinleştirmesi (SystemdService=) kabuk adı almadan önce gelen ilk bildirimde onları yeniden
#   başlatıp adı kapardı. Kullanıcının kendi birim dosyası (~/.config/systemd/user) maskelenemez;
#   yalnız devre dışı bırakılır.
# - systemd dışında çalışan süreç (ör. Hyprland exec-once): yalnız bu kullanıcının, tam adla
#   (pgrep/pkill -x -u) SIGTERM. Hyprland ayarındaki exec-once satırına DOKUNULMAZ (ana ayarın
#   devralınması ADR-0007'nin işi); satır hâlâ duruyorsa yeri yazdırılır.
# - restore: maskeyi kaldırır, önceden etkin olanı etkinleştirir, çalışanı başlatır; exec-once
#   süreçleri sonraki girişte kendi satırlarından döner.
# Çıkış: 0 tamam, 1 hata, 2 kullanım.
set -eu

STATE_DIR=${XDG_STATE_HOME:-${HOME:?HOME gerekli}/.local/state}/nuxut/takeover
RECORD=$STATE_DIR/services
CONF=${XDG_CONFIG_HOME:-$HOME/.config}
NOTIFY_DAEMONS="dunst mako swaync"
SHELLS="waybar quickshell qs ags eww"
UID_=$(id -u)

say() { printf '%s\n' "$*"; }

usage() {
    echo "kullanım: conflicting-services.sh apply [--now] | restore | plan" >&2
    exit 2
}

is_notify() { case " $NOTIFY_DAEMONS " in *" $1 "*) return 0 ;; esac; return 1; }

# Kullanıcı birimi var mı (dosya ya da yüklü birim); varsa "durum etkin_mi" yazar.
unit_state() { # unit_state AD → "<UnitFileState> <ActiveState>" ya da boş
    out=$(systemctl --user show -p LoadState -p UnitFileState -p ActiveState "$1.service" 2>/dev/null) || return 0
    load=$(printf '%s\n' "$out" | sed -n 's/^LoadState=//p')
    [ "$load" = loaded ] || [ "$load" = masked ] || return 0
    file=$(printf '%s\n' "$out" | sed -n 's/^UnitFileState=//p')
    active=$(printf '%s\n' "$out" | sed -n 's/^ActiveState=//p')
    printf '%s %s\n' "${file:-unknown}" "${active:-inactive}"
}

user_unit_file() { [ -e "$CONF/systemd/user/$1.service" ] || [ -L "$CONF/systemd/user/$1.service" ]; }

recorded() { [ -f "$RECORD" ] && grep -q "^$1 $2 " "$RECORD"; }

record() { # record SATIR — dosyaya fsync'li ekleme yerine: geçici + yeniden adlandırma
    mkdir -p "$STATE_DIR"
    tmp=$RECORD.yeni.$$
    { [ -f "$RECORD" ] && cat "$RECORD"; printf '%s\n' "$1"; } > "$tmp"
    mv -f "$tmp" "$RECORD"
}

# Hyprland ayarında bu programı oturum başında başlatan satırlar (yorum değil): dosya:satır.
# hyprlang: exec-once/exec/execr-once/execr = …; Lua: exec içeren çağrı satırı.
exec_lines() { # exec_lines AD
    [ -d "$CONF/hypr" ] || return 0
    w="(^|[^[:alnum:]_.-])$1([^[:alnum:]_.-]|\$)"
    find "$CONF/hypr" -maxdepth 3 -type f \( -name '*.conf' -o -name '*.lua' \) 2>/dev/null | while IFS= read -r f; do
        case $f in
            *.conf) re="^[[:space:]]*(exec-once|exec|execr-once|execr)[[:space:]]*=.*$w" ;;
            *) re="^[[:space:]]*[^-[:space:]].*exec.*$w" ;;
        esac
        grep -nE "$re" "$f" 2>/dev/null | cut -d: -f1 | while IFS= read -r n; do printf '%s:%s\n' "$f" "$n"; done
    done
}

plan_or_apply() { # $1: plan|apply
    mode=$1
    did=0
    for p in $NOTIFY_DAEMONS $SHELLS; do
        st=$(unit_state "$p")
        if [ -n "$st" ]; then
            file=${st% *}; active=${st#* }
            # Bar: etkin ya da çalışıyorsa. Bildirim servisi: maskeli değilse her zaman (statik
            # birim de D-Bus'la etkinleşir).
            want=0
            case $file in enabled|enabled-runtime|alias|linked|linked-runtime) want=1 ;; esac
            case $active in active|activating) want=1 ;; esac
            is_notify "$p" && [ "$file" != masked ] && want=1
            if [ "$want" = 1 ] && ! recorded unit "$p"; then
                mask=0
                is_notify "$p" && ! user_unit_file "$p" && [ "$file" != masked ] && mask=1
                if [ "$mode" = plan ]; then
                    extra=""; [ "$mask" = 1 ] && extra=", maskelenirdi (D-Bus etkinleştirmesi)"
                    say "$p.service: durdurulup devre dışı bırakılırdı$extra (şimdi: $file, $active)"
                else
                    record "unit $p $file $active $mask"
                    if [ "$NOW" = 1 ]; then
                        systemctl --user disable --now "$p.service" >/dev/null 2>&1 || true
                    else
                        systemctl --user disable "$p.service" >/dev/null 2>&1 || true
                    fi
                    if [ "$mask" = 1 ]; then
                        systemctl --user mask "$p.service" >/dev/null 2>&1 \
                            || say "$p.service maskelenemedi (D-Bus etkinleştirmesi açık kalabilir)"
                    fi
                    say "$p.service devralındı (önce: $file, $active)"
                fi
                did=1
            fi
        fi
        if pgrep -x -u "$UID_" "$p" >/dev/null 2>&1; then
            if [ "$mode" = plan ]; then
                say "$p çalışıyor: kapatılırdı (SIGTERM; yalnız Nuxut kabuğu çalışacaksa)"
            elif [ "$NOW" = 0 ]; then
                say "$p çalışıyor: Nuxut kabuğu çalışmadığı için bu oturumda kapatılmadı"
            else
                recorded proc "$p" || record "proc $p - - -"
                pkill -TERM -x -u "$UID_" "$p" 2>/dev/null || true
                say "$p kapatıldı"
            fi
            did=1
        fi
        exec_lines "$p" | while IFS= read -r l; do
            say "$p Hyprland ayarında başlatılıyor ($l): satırı kaldırın, yoksa sonraki girişte yine açılır"
        done
    done
    [ "$did" = 1 ] || say "çakışan bar/bildirim servisi yok"
}

restore() {
    if [ ! -f "$RECORD" ]; then
        say "geri getirilecek servis kaydı yok"
        return 0
    fi
    while read -r kind p file active mask; do
        case $kind in
            unit)
                [ "$mask" = 1 ] && { systemctl --user unmask "$p.service" >/dev/null 2>&1 || say "$p.service maskesi kaldırılamadı"; }
                case $file in
                    enabled|enabled-runtime|alias|linked|linked-runtime)
                        systemctl --user enable "$p.service" >/dev/null 2>&1 || say "$p.service etkinleştirilemedi" ;;
                esac
                if [ "$active" = active ] || [ "$active" = activating ]; then
                    systemctl --user start "$p.service" >/dev/null 2>&1 || say "$p.service başlatılamadı"
                fi
                say "$p.service geri getirildi ($file, $active)"
                ;;
            proc)
                say "$p: Hyprland exec-once satırından sonraki girişte yeniden başlar"
                ;;
        esac
    done < "$RECORD"
    rm -f "$RECORD"
    rmdir "$STATE_DIR" 2>/dev/null || true
}

NOW=0
[ $# = 2 ] && [ "$1" = apply ] && [ "$2" = --now ] && { NOW=1; set -- apply; }
[ $# = 1 ] || usage
case $1 in
    apply) plan_or_apply apply ;;
    plan) plan_or_apply plan ;;
    restore) restore ;;
    *) usage ;;
esac
