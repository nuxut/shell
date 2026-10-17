# Girdi tuzağı (kendi kitty penceremizin içinde çalışır): terminali ham kipe alır, gelen her
# baytı $1'e yazar. Kilit açıkken bu pencereye hiçbir tuş ulaşmamalı.
set -eu
stty -icanon -echo min 1
exec cat > "$1"
