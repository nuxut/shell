#!/bin/sh
# install.sh PATH denetimi (tests/it/install_path.rs çalıştırır): "--- PATH" bölümü (ve güven
# yardımcıları path_trusted/dir_private) ayıklanıp sahte hyprctl/systemctl ile çalıştırılır; gerçek
# Hyprland oturumuna ve systemd'ye ulaşılmaz. Lua parçası gerçek `lua` ile (hl.env sahte) ayrıca
# denenir. Güvenlik (2026-10-17): dizin PATH'in SONUNA, yalnız güvenilir ve yalın yolsa eklenir.
# Kullanım: install_path.sh <install.sh yolu>
set -eu
NEW=$1
# Gerçek hyprctl hiçbir senaryoda kullanıcının oturumuna ulaşamaz: XDG_RUNTIME_DIR geçici dizin.
T=$(mktemp -d -p "${NUXUT_TEST_TMP:?NUXUT_TEST_TMP gerekli (paylaşımsız geçici dizin)}")
trap 'chmod -R u+w "$T" 2>/dev/null; rm -rf "$T"' EXIT
fail() { echo "BAŞARISIZ: $*"; exit 1; }

{
  sed -n '/^path_trusted() {/,/^}/p' "$NEW"
  sed -n '/^dir_private() {/,/^}/p' "$NEW"
  sed -n '/^# --- PATH/,/^# --- Kabuğu yeniden başlatma/p' "$NEW"
} > "$T/block.sh"
grep -q '^hypr_path_live()' "$T/block.sh" || fail "PATH bölümü bulunamadı"
grep -q '^path_trusted()' "$T/block.sh" && grep -q '^dir_private()' "$T/block.sh" || fail "güven yardımcıları bulunamadı"

# Güvenilir kurulum dizini: kullanıcıya ait, 0755, sembolik bağ değil (üst dizinler 0700).
GOOD=$T/home/.local/bin
mkdir -p "$GOOD"
chmod 700 "$T/home" "$T/home/.local"
chmod 755 "$GOOD"

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

# $1 ad, $2 FAKE_HYPR, $3 imza (boş = Hyprland dışı), $4 oturum PATH'i, $5 terminal PATH'i;
# BIN ortamdan (varsayılan güvenilir dizin).
scenario() {
  rm -f "$T/hyprctl.log"
  env -i HOME="$T/home" XDG_RUNTIME_DIR="$T" PATH="$T/bin:/usr/bin:/bin" FAKEBIN="$T/bin" DRY="${DRY:-0}" FAKE_HYPR="$2" HYPRLAND_INSTANCE_SIGNATURE="$3" \
    NUXUT_SESSION_PATH="$4" TERM_PATH="$5" BLOCK="$T/block.sh" TBIN="${BIN:-$GOOD}" \
    sh -c 'note() { printf "    %s\n" "$*"; }; warn() { printf "!! %s\n" "$*"; }
           BIN=$TBIN CONF=$HOME/.config DRY_RUN=${DRY:-0} FIX_PATH=0
           [ -n "$HYPRLAND_INSTANCE_SIGNATURE" ] || unset HYPRLAND_INSTANCE_SIGNATURE
           . "$BLOCK"; PATH="$FAKEBIN:$TERM_PATH:/usr/bin:/bin"; path_check' > "$T/$1.out" 2>&1 || fail "$1: $(cat "$T/$1.out")"
}

# 1) Lua ayarı: eval ile SONA eklenir, ileti yeni terminalleri söyler.
scenario lua lua sig /usr/bin /usr/bin
grep -qF "çalışan Hyprland oturumunun PATH'ine eklendi" "$T/lua.out" || fail "lua: $(cat "$T/lua.out")"
grep -qF "hl.env(\"PATH\", p .. \":$GOOD\", true)" "$T/hyprctl.log" || fail "lua: eval sona eklemiyor: $(cat "$T/hyprctl.log")"
echo "1 tamam: Lua oturumuna eval ile sona eklendi"

# 2) hyprlang ayarı: eval reddedilir, keyword envd ile SONA eklenir ($PATH'i Hyprland açar).
scenario legacy legacy sig /usr/bin /usr/bin
grep -qxF "keyword envd PATH,\$PATH:$GOOD" "$T/hyprctl.log" || fail "legacy: $(cat "$T/hyprctl.log")"
grep -qF "eklendi" "$T/legacy.out" || fail "legacy: $(cat "$T/legacy.out")"
echo "2 tamam: hyprlang oturumuna keyword envd ile sona eklendi"

# 3) Hyprland dışı: IPC yok, yeniden giriş iletisi.
scenario nohypr lua "" /usr/bin /usr/bin
[ ! -e "$T/hyprctl.log" ] || fail "nohypr: hyprctl çağrıldı"
grep -qF "oturumu yeniden açın" "$T/nohypr.out" || fail "nohypr: $(cat "$T/nohypr.out")"
echo "3 tamam: Hyprland dışında yeniden giriş iletisi"

# 4) Oturumda zaten var, bu terminalde yok: IPC yok, yeni terminaller iletisi.
scenario session lua sig "/usr/bin:$GOOD" /usr/bin
[ ! -e "$T/hyprctl.log" ] || fail "session: hyprctl çağrıldı"
grep -qF "yeni açılan terminallerde nuxut-shell çalışır" "$T/session.out" || fail "session: $(cat "$T/session.out")"
echo "4 tamam: oturumda varken IPC yok"

# 5) Deneme: IPC yok, plan satırı.
DRY=1 scenario dry lua sig /usr/bin /usr/bin
[ ! -e "$T/hyprctl.log" ] || fail "dry: hyprctl çağrıldı"
grep -qF "[deneme]" "$T/dry.out" || fail "dry: $(cat "$T/dry.out")"
echo "5 tamam: denemede IPC yok"

# 6) Güvenilmeyen dizin: grup/başkaları yazabilir, sembolik bağ, üst dizin yazılabilir, başka
#    kök, enjeksiyon karakterleri → hyprctl hiç çağrılmaz, uyarı.
untrusted() { # $1 ad, $2 dizin
  BIN=$2 scenario "$1" lua sig /usr/bin /usr/bin
  [ ! -e "$T/hyprctl.log" ] || fail "$1: hyprctl çağrıldı: $(cat "$T/hyprctl.log")"
  grep -qF "güvenilir değil" "$T/$1.out" || fail "$1: $(cat "$T/$1.out")"
}
chmod 775 "$GOOD"; untrusted group_writable "$GOOD"; chmod 755 "$GOOD"
chmod 757 "$GOOD"; untrusted other_writable "$GOOD"; chmod 755 "$GOOD"
chmod 770 "$T/home/.local"; untrusted parent_writable "$GOOD"; chmod 700 "$T/home/.local"
ln -s "$GOOD" "$T/home/link"; untrusted symlink "$T/home/link"
for bad in 'a,b' 'a$b' 'a b' 'a:b'; do
  mkdir -p "$T/home/$bad"; chmod 755 "$T/home/$bad"
done
untrusted comma "$T/home/a,b"
untrusted dollar "$T/home/a\$b"
untrusted space "$T/home/a b"
untrusted colon "$T/home/a:b"
untrusted dotdot "$T/home/.local/../.local/bin"
untrusted relative "home/.local/bin"
echo "6 tamam: güvenilmeyen dizin ve enjeksiyon karakterleri reddedildi"

# 7) environment.d satırı: sona; güvenilmeyende boş ve hata.
line=$(env -i HOME="$T/home" BLOCK="$T/block.sh" TBIN="$GOOD" sh -c 'BIN=$TBIN CONF=$HOME/.config; . "$BLOCK"; envd_line')
[ "$line" = "PATH=\${PATH}:$GOOD" ] || fail "envd_line: [$line]"
if env -i HOME="$T/home" BLOCK="$T/block.sh" TBIN="$T/home/a,b" sh -c 'BIN=$TBIN CONF=$HOME/.config; . "$BLOCK"; envd_line' >/dev/null; then
  fail "envd_line: güvensiz yol kabul edildi"
fi
echo "7 tamam: environment.d satırı sona, güvensiz yol reddedildi"

# 8) Lua parçası gerçek yorumlayıcıda: yoksa bir kez SONA ekler, varsa eklemez.
if command -v lua >/dev/null 2>&1; then
  lua_code=$(grep -o 'local p = os.getenv.*end"$' "$T/block.sh" | sed 's/"$//; s/\\"/"/g; s/\$BIN/\/home\/u\/.local\/bin/g')
  stub='hl = { env = function(k, v, d) print("ENV " .. k .. "=" .. v .. " " .. tostring(d)) end }'
  out=$(PATH=/usr/bin:/bin lua -e "$stub" -e "$lua_code")
  [ "$out" = "ENV PATH=/usr/bin:/bin:/home/u/.local/bin true" ] || fail "lua yorumlayıcı: [$out] ($lua_code)"
  out=$(PATH=/home/u/.local/bin:/usr/bin lua -e "$stub" -e "$lua_code")
  [ -z "$out" ] || fail "lua yorumlayıcı: varken eklendi: $out"
  echo "8 tamam: Lua parçası doğru"
fi

echo "HEPSİ TAMAM"
