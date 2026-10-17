#!/bin/sh
# install.sh PATH denetimi (tests/it/install_path.rs çalıştırır): "--- PATH" bölümü ayıklanıp sahte
# hyprctl/systemctl ile çalıştırılır; gerçek Hyprland oturumuna ve systemd'ye ulaşılmaz. Lua
# parçası gerçek `lua` ile (hl.env sahte) ayrıca denenir.
# Kullanım: install_path.sh <install.sh yolu>
set -eu
NEW=$1
# Gerçek hyprctl hiçbir senaryoda kullanıcının oturumuna ulaşamaz: XDG_RUNTIME_DIR geçici dizin.
T=$(mktemp -d -p "${NUXUT_TEST_TMP:?NUXUT_TEST_TMP gerekli (paylaşımsız geçici dizin)}")
trap 'rm -rf "$T"' EXIT
fail() { echo "BAŞARISIZ: $*"; exit 1; }

sed -n '/^# --- PATH/,/^# --- Kabuğu yeniden başlatma/p' "$NEW" > "$T/block.sh"
grep -q '^hypr_path_live()' "$T/block.sh" || fail "PATH bölümü bulunamadı"

mkdir -p "$T/bin"
# Sahte hyprctl: argümanları yazar; FAKE_HYPR=lua → eval "ok"; legacy → eval reddedilir, keyword "ok".
cat > "$T/bin/hyprctl" <<EOF
#!/bin/sh
printf '%s\n' "\$*" >> "$T/hyprctl.log"
case \$1 in
  eval) if [ "\$FAKE_HYPR" = lua ]; then echo ok; else echo "eval is only supported with the lua config manager"; fi ;;
  keyword) echo ok ;;
  *) exit 1 ;;
esac
EOF
chmod +x "$T/bin/hyprctl"

# $1 ad, $2 FAKE_HYPR, $3 imza (boş = Hyprland dışı), $4 oturum PATH'i, $5 terminal PATH'i
scenario() {
  rm -f "$T/hyprctl.log"
  env -i HOME="$T" XDG_RUNTIME_DIR="$T" PATH="$T/bin:/usr/bin:/bin" FAKEBIN="$T/bin" DRY="${DRY:-0}" FAKE_HYPR="$2" HYPRLAND_INSTANCE_SIGNATURE="$3" \
    NUXUT_SESSION_PATH="$4" TERM_PATH="$5" BLOCK="$T/block.sh" \
    sh -c 'note() { printf "    %s\n" "$*"; }; warn() { printf "!! %s\n" "$*"; }
           BIN=/home/u/.local/bin CONF=/home/u/.config DRY_RUN=${DRY:-0} FIX_PATH=0
           [ -n "$HYPRLAND_INSTANCE_SIGNATURE" ] || unset HYPRLAND_INSTANCE_SIGNATURE
           . "$BLOCK"; PATH="$FAKEBIN:$TERM_PATH:/usr/bin:/bin"; path_check' > "$T/$1.out" 2>&1 || fail "$1: $(cat "$T/$1.out")"
}

# 1) Lua ayarı: eval ile eklenir, ileti yeni terminalleri söyler.
scenario lua lua sig /usr/bin /usr/bin
grep -qF "çalışan Hyprland oturumunun PATH'ine eklendi" "$T/lua.out" || fail "lua: $(cat "$T/lua.out")"
grep -q '^eval .*hl.env("PATH", "/home/u/.local/bin:"' "$T/hyprctl.log" || fail "lua: eval yok: $(cat "$T/hyprctl.log")"
echo "1 tamam: Lua oturumuna eval ile eklendi"

# 2) hyprlang ayarı: eval reddedilir, keyword envd ile eklenir ($PATH'i Hyprland açar).
scenario legacy legacy sig /usr/bin /usr/bin
grep -qxF 'keyword envd PATH,/home/u/.local/bin:$PATH' "$T/hyprctl.log" || fail "legacy: $(cat "$T/hyprctl.log")"
grep -qF "eklendi" "$T/legacy.out" || fail "legacy: $(cat "$T/legacy.out")"
echo "2 tamam: hyprlang oturumuna keyword envd ile eklendi"

# 3) Hyprland dışı: IPC yok, yeniden giriş iletisi.
scenario nohypr lua "" /usr/bin /usr/bin
[ ! -e "$T/hyprctl.log" ] || fail "nohypr: hyprctl çağrıldı"
grep -qF "oturumu yeniden açın" "$T/nohypr.out" || fail "nohypr: $(cat "$T/nohypr.out")"
echo "3 tamam: Hyprland dışında yeniden giriş iletisi"

# 4) Oturumda zaten var, bu terminalde yok: IPC yok, yeni terminaller iletisi.
scenario session lua sig /home/u/.local/bin:/usr/bin /usr/bin
[ ! -e "$T/hyprctl.log" ] || fail "session: hyprctl çağrıldı"
grep -qF "yeni açılan terminallerde nuxut-shell çalışır" "$T/session.out" || fail "session: $(cat "$T/session.out")"
echo "4 tamam: oturumda varken IPC yok"

# 5) Deneme: IPC yok, plan satırı.
DRY=1 scenario dry lua sig /usr/bin /usr/bin
[ ! -e "$T/hyprctl.log" ] || fail "dry: hyprctl çağrıldı"
grep -qF "[deneme]" "$T/dry.out" || fail "dry: $(cat "$T/dry.out")"
echo "5 tamam: denemede IPC yok"

# 6) Lua parçası gerçek yorumlayıcıda: yoksa bir kez ekler, varsa eklemez.
if command -v lua >/dev/null 2>&1; then
  lua_code=$(grep -o 'local p = os.getenv.*end"$' "$T/block.sh" | sed 's/"$//; s/\\"/"/g; s/\$BIN/\/home\/u\/.local\/bin/g')
  stub='hl = { env = function(k, v, d) print("ENV " .. k .. "=" .. v .. " " .. tostring(d)) end }'
  out=$(PATH=/usr/bin:/bin lua -e "$stub" -e "$lua_code")
  [ "$out" = "ENV PATH=/home/u/.local/bin:/usr/bin:/bin true" ] || fail "lua yorumlayıcı: [$out] ($lua_code)"
  out=$(PATH=/home/u/.local/bin:/usr/bin lua -e "$stub" -e "$lua_code")
  [ -z "$out" ] || fail "lua yorumlayıcı: varken eklendi: $out"
  echo "6 tamam: Lua parçası doğru"
fi

echo "HEPSİ TAMAM"
