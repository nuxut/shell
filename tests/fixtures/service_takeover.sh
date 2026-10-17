#!/bin/sh
# dist/takeover/conflicting-services.sh senaryosu (tests/it/service_takeover.rs çalıştırır): geçici
# HOME/XDG dizinleri, sahte systemctl/pgrep/pkill (PATH başında; gerçek birimlere ve süreçlere
# ulaşılmaz).
# Kullanım: service_takeover.sh <conflicting-services.sh yolu>
set -eu
SCRIPT=$1
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
export HOME="$T/home" XDG_CONFIG_HOME="$T/home/.config" XDG_STATE_HOME="$T/home/.local/state"
F="$T/fake"
mkdir -p "$HOME" "$F/units" "$F/procs" "$F/bin" "$XDG_CONFIG_HOME/hypr" "$XDG_CONFIG_HOME/systemd/user"
fail() { echo "BAŞARISIZ: $*"; exit 1; }

# Sahte systemctl: birim durumu $F/units/<ad> ("UnitFileState ActiveState"), satıcı durumu .base.
cat > "$F/bin/systemctl" <<SH
#!/bin/sh
echo "\$*" >> "$F/systemctl.log"
[ "\$1" = --user ] && shift
cmd=\$1; shift
case \$cmd in
  show)
    while [ \$# -gt 0 ]; do case \$1 in -p) shift 2 ;; *) u=\${1%.service}; shift ;; esac; done
    if [ -f "$F/units/\$u" ]; then
      read -r file active < "$F/units/\$u"
      ld=loaded; [ "\$file" = masked ] && ld=masked
      printf 'LoadState=%s\nUnitFileState=%s\nActiveState=%s\n' "\$ld" "\$file" "\$active"
    else
      printf 'LoadState=not-found\nUnitFileState=\nActiveState=inactive\n'
    fi ;;
  disable)
    [ "\$1" = --now ] && { now=1; shift; } || now=0
    u=\${1%.service}; read -r file active < "$F/units/\$u"
    case \$file in enabled*|alias|linked*) file=\$(cat "$F/units/\$u.base") ;; esac
    [ \$now = 1 ] && active=inactive
    echo "\$file \$active" > "$F/units/\$u" ;;
  mask) u=\${1%.service}; read -r file active < "$F/units/\$u"; echo "masked \$active" > "$F/units/\$u" ;;
  unmask) u=\${1%.service}; read -r file active < "$F/units/\$u"; echo "\$(cat "$F/units/\$u.base") \$active" > "$F/units/\$u" ;;
  enable) u=\${1%.service}; read -r file active < "$F/units/\$u"; echo "enabled \$active" > "$F/units/\$u" ;;
  start) u=\${1%.service}; read -r file active < "$F/units/\$u"; echo "\$file active" > "$F/units/\$u" ;;
  *) exit 1 ;;
esac
SH
# Sahte pgrep/pkill: $F/procs/<ad> varsa süreç "çalışıyor".
printf '#!/bin/sh\nfor a; do n=$a; done\n[ -e "%s/procs/$n" ]\n' "$F" > "$F/bin/pgrep"
printf '#!/bin/sh\necho "pkill $*" >> "%s/systemctl.log"\nfor a; do n=$a; done\nrm -f "%s/procs/$n"\n' "$F" "$F" > "$F/bin/pkill"
chmod +x "$F/bin/"*
export PATH="$F/bin:$PATH"

unit() { echo "$2 $3" > "$F/units/$1"; echo "$4" > "$F/units/$1.base"; } # ad dosya-durumu etkinlik satıcı-durumu
unit dunst static inactive static          # D-Bus ile etkinleşen bildirim servisi
unit waybar enabled active disabled        # etkin bar
unit swaync enabled active disabled        # kullanıcının kendi birim dosyası: maskelenemez
touch "$XDG_CONFIG_HOME/systemd/user/swaync.service"
unit ags disabled inactive disabled        # kurulu ama kullanılmıyor: dokunulmaz
touch "$F/procs/eww"                       # exec-once ile çalışan
printf 'monitor = ,preferred,auto,1\n# exec-once = waybar\nexec-once = eww daemon\nbind = SUPER, E, exec, ewwx\n' > "$XDG_CONFIG_HOME/hypr/hyprland.conf"
REC="$XDG_STATE_HOME/nuxut/takeover/services"

# 1) plan: hiçbir şey değişmez.
sh "$SCRIPT" plan > "$T/o1"
[ ! -e "$REC" ] || fail "1: plan kayıt yazdı"
[ ! -e "$F/systemctl.log" ] || ! grep -qvE '^--user show' "$F/systemctl.log" || fail "1: plan systemctl ile değiştirdi"
grep -q 'dunst.service: durdurulup devre dışı bırakılırdı, maskelenirdi' "$T/o1" || { cat "$T/o1"; fail "1: dunst planı"; }
grep -q 'eww çalışıyor' "$T/o1" || fail "1: eww planı"
grep -q 'ags' "$T/o1" && fail "1: kullanılmayan ags planda"
echo "1 tamam: plan hiçbir şey değiştirmez"

# 2) apply: önce kayıt, sonra devre dışı/maske/kapatma.
sh "$SCRIPT" apply --now > "$T/o2"
[ "$(cat "$F/units/dunst")" = "masked inactive" ] || fail "2: dunst maskelenmedi"
[ "$(cat "$F/units/waybar")" = "disabled inactive" ] || fail "2: waybar devre dışı değil"
[ "$(cat "$F/units/swaync")" = "disabled inactive" ] || fail "2: swaync devre dışı değil"
grep -q 'mask swaync' "$F/systemctl.log" && fail "2: kullanıcının birim dosyası maskelendi"
[ "$(cat "$F/units/ags")" = "disabled inactive" ] || fail "2: ags'e dokunuldu"
[ ! -e "$F/procs/eww" ] || fail "2: eww kapatılmadı"
grep -q -- '-x -u' "$F/systemctl.log" || fail "2: pkill tam ad/kullanıcı ile değil"
grep -qF "hyprland.conf:3)" "$T/o2" || { cat "$T/o2"; fail "2: eww exec-once satırı bildirilmedi"; }
grep -qF "hyprland.conf:2" "$T/o2" && fail "2: yorum satırı bildirildi"
grep -qF "hyprland.conf:4" "$T/o2" && fail "2: kısayol satırı bildirildi"
grep -qx 'unit dunst static inactive 1' "$REC" || { cat "$REC"; fail "2: dunst kaydı"; }
grep -qx 'unit waybar enabled active 0' "$REC" || fail "2: waybar kaydı"
grep -qx 'unit swaync enabled active 0' "$REC" || fail "2: swaync kaydı"
grep -qx 'proc eww - - -' "$REC" || fail "2: eww kaydı"
echo "2 tamam: devralındı, kayıt özgün durumu taşıyor"

# 3) İkinci apply özgün kaydı ezmez.
cp "$REC" "$T/rec2"
sh "$SCRIPT" apply --now > "$T/o3"
cmp -s "$REC" "$T/rec2" || fail "3: kayıt değişti"
echo "3 tamam: ikinci apply özgün kaydı korur"

# 4) restore: her şey eski hâline, kayıt silinir.
sh "$SCRIPT" restore > "$T/o4"
[ "$(cat "$F/units/dunst")" = "static inactive" ] || fail "4: dunst maskesi kalktı mı: $(cat "$F/units/dunst")"
[ "$(cat "$F/units/waybar")" = "enabled active" ] || fail "4: waybar: $(cat "$F/units/waybar")"
[ "$(cat "$F/units/swaync")" = "enabled active" ] || fail "4: swaync: $(cat "$F/units/swaync")"
[ ! -e "$REC" ] || fail "4: kayıt kaldı"
grep -q 'eww: Hyprland exec-once' "$T/o4" || fail "4: eww notu"
sh "$SCRIPT" restore | grep -q 'kaydı yok' || fail "4: ikinci restore"
echo "4 tamam: geri getirildi"

# 5) Nuxut kabuğu çalışmayacaksa (--now yok): birimler devre dışı/maskeli ama bu oturumda hiçbir
#    şey durdurulmaz (kullanıcı barsız kalmasın).
touch "$F/procs/eww"
sh "$SCRIPT" apply > "$T/o5"
[ "$(cat "$F/units/waybar")" = "disabled active" ] || fail "5: waybar: $(cat "$F/units/waybar")"
[ "$(cat "$F/units/dunst")" = "masked inactive" ] || fail "5: dunst maskelenmedi"
[ -e "$F/procs/eww" ] || fail "5: eww kapatıldı"
grep -q "eww çalışıyor: Nuxut kabuğu çalışmadığı" "$T/o5" || fail "5: eww notu"
sh "$SCRIPT" restore > /dev/null
[ "$(cat "$F/units/waybar")" = "enabled active" ] || fail "5: geri getirme"
echo "5 tamam: --now olmadan bu oturumda hiçbir şey durmaz"

echo HEPSİ TAMAM
