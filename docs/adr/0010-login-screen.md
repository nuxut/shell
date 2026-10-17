# ADR-0010: Giriş ekranı — greetd + `nuxut-greeter`

- Durum: kabul edildi (2026-10-16; kullanıcı sorusu "SDDM varsayılan desteklediğimiz mi olacak,
  yoksa başka bir şey mi? Doğru şekilde yapalım."). SDDM kullanılmaz (2026-10-17).
- İlgili: `docs/design/greeter.md` (akış, tehdit modeli, kurulum/geri alma, test), ARCHITECTURE
  "Kilit ekranı" (görünüm ve kimlik doğrulama kuralları ortak), ADR-0003 (Hyprland Lua ayarı).

## Bağlam

Nuxut bugün tty'den `start-hyprland` ile açılıyor; giriş ekranı yok. Tam bir masaüstünde giriş →
kilit → masaüstü aynı görsel dili konuşmalı: kilit ekranı (`nuxut-lock`) zaten Rust'ta,
framework bileşenleriyle, PAM ve parmak izi akışıyla var. Soru, giriş ekranının hangi display
manager üstünde ve hangi dille yazılacağı.

## Seçenekler

| Seçenek | Görünüm | Wayland | Güvenlik / yalıtım | Nuxut'a uyumu |
|---|---|---|---|---|
| **SDDM** | QML teması | Wayland greeter'ı deneysel/kompozitöre bağlı (KWin ya da Weston) | Büyük C++ yüzeyi, greeter `sddm` kullanıcısıyla | Görünüm QML'de ikinci kez yazılır; framework bileşenleri, tema rolleri, hareket kullanılamaz; Qt bağımlılığı |
| **GDM** | GNOME Shell | Tam | Olgun | GNOME Shell'in kendisi; tema değiştirilemez, ağır |
| **LightDM** | GTK/webkit greeter'ları | X11 merkezli | Orta | Wayland'de zayıf |
| **ly** | TUI | Hayır (tty) | Küçük | Grafik değil |
| **greetd + tuigreet** | TUI | — | Küçük | Grafik değil |
| **greetd + regreet** | GTK4 (Rust) | cage içinde | greeter ayrı kullanıcı | GTK teması; Nuxut görünümü değil |
| **greetd + gtkgreet** | GTK3 | katman yüzeyi | ayrı kullanıcı | GTK3, bakımı zayıf |
| **greetd + `nuxut-greeter`** | Nuxut framework | Hyprland içinde katman yüzeyleri | greeter ayrı kullanıcı; PAM root'taki greetd'de | Kilitle aynı kod ve görünüm |

greetd: küçük (Rust), Wayland'in standart giriş servisi; PAM'i root olarak kendisi yapar, greeter
yalnızca bir Unix soketi üzerinden JSON konuşur (`greetd-ipc(7)`). Greeter'ın parola dışında hiçbir
ayrıcalığı yoktur.

## Karar

1. **Display manager: greetd.** SDDM/GDM değil.
2. **Giriş ekranı: kendi `nuxut-greeter`'ımız** (Rust, `nuxut-framework`), bu repoda `greeter/`
   çalışma alanı üyesi. Kilit ekranıyla ortak: görünüm (`nuxut_lock::view::OutputView`: duvar
   kağıdı, karartma, marka işareti, saat, tarih), durum satırları ve hata metinleri
   (`status_lines`), artan gecikme (`backoff_for`), yalıtılmış avatar/duvar kağıdı çözümü
   (`LockImages`), `lock.toml` ayarları, `SecretBuf` (mlock'lu, sıfırlanan tampon) ve
   `PR_SET_DUMPABLE=0`. Giriş → kilit → masaüstü aynı görünümle akar.
3. **Compositor: Hyprland**, greeter'a özel küçük bir Lua ayarıyla (`dist/greeter/hyprland.lua`).

   | | Hyprland | cage | sway |
   |---|---|---|---|
   | Kurulu mu | Evet (Nuxut'un compositor'ı) | Ek paket | Ek paket |
   | Katman yüzeyi (çıkış başına arka plan + form) | Var | Yok (tek tam ekran pencere) | Var |
   | Çoklu monitör | Her çıkışa yüzey; ölçek `auto` (PPI) | Tek uygulama, `-m extend/last` | Var |
   | HiDPI | Çıkış başına kesirli ölçek | Ayar yok | Ayar dosyasıyla |
   | NVIDIA + Intel Arc (çoklu GPU) | Oturumla **aynı** sürücü yolu (aquamarine, açık senkron, `AQ_DRM_DEVICES`) | wlroots; NVIDIA'da ayrıca sınanmalı | wlroots; NVIDIA resmî desteklenmiyor |
   | Açılış hızı | Daha ağır (ölçülmedi; tahmin: cage'den yavaş) | En hafif | Hafif |

   Belirleyici olan: kullanıcının donanımında masaüstüyle **aynı** compositor çalışır (giriş ekranı
   açılıyorsa masaüstü de açılır; tersi cage/sway ile garanti değil) ve çıkış başına katman yüzeyi
   kilit ekranıyla aynı yerleşimi verir. Hyprland'in hiç bind kayıtlı değilken açtığı "acil durum"
   bind'leri (SUPER+Q terminal, SUPER+R çalıştırıcı) greeter ayarında dosyanın **ilk satırında**
   zararsız bir bind ile kapatılır (`docs/design/greeter.md` "Tehdit modeli").
4. **Sistem değişikliği yalnız kullanıcı onayıyla.** greetd'yi etkinleştirmek,
   `display-manager.service`'i değiştirmek root işidir: kök yetkili sistem yardımcısının `greeter`
   alanı (ADR-0008), Hoş geldin'in "Sistem görünümünü Nuxut'a geçir" adımında (seçili gelir) ya da
   `install.sh --system-theme` ile, `pkexec` üzerinden; yedekli ve geri alınabilir. Bugünkü tty +
   `start-hyprland` düzeni geri alma ile korunur.

## Sonuçlar

- Artı: tek görsel dil; kilit ve giriş aynı kodu paylaşır (güvenlik düzeltmesi ikisine birden
  gider); PAM greetd'de, greeter ayrıcalıksız.
- Eksi: Hyprland'in açılışı cage'den ağır; greetd paketi kurulmalı; greeter kullanıcının ev dizinini
  okuyamadığı için tema ve duvar kağıdı sisteme **yayımlanmalı** (`/var/lib/nuxut-greeter/config`).
- greetd protokolü katı istek-yanıttır: PAM bir modülde beklerken (ör. `pam_fprintd` parmak
  bekliyor) greeter araya giremez. Parmak izi greetd'nin PAM yığınına eklenirse önce o beklenir;
  bu yüzden giriş varsayılanı şifredir (kilit ekranı parmak izini fprintd D-Bus ile paralel yapar).
- cage/sway desteği (katman yüzeyi olmayan ortamda tek pencere) açık iş; sürüm 1'de yok.
