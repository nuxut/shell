# ADR 0008: Kök yetkili sistem yardımcısı (`nuxut-system-helper`)

- Durum: kabul edildi (2026-10-16).
- İlgili: `docs/design/boot-theme.md` (ilk alan: önyükleme teması), giriş ekranı
  (ikinci alan, ayrı iş), `docs/design/polkit.md` (polkit ajanı).

## Bağlam

Nuxut'un "tam masaüstü" hedefi sistem dosyalarına dokunmayı gerektiren işler içeriyor: Plymouth
teması (`/usr/share`, `/etc/plymouth`, initramfs), önyükleyici teması (`/boot`, `/etc/default`,
ESP), giriş ekranı (`/etc/greetd`). Kabuk ve Ayarlar sisteme yazmaz; kullanıcı onayıyla, denetlenmiş
küçük bir bileşen yazar. Seçenekler:

1. **D-Bus sistem servisi** (`org.nuxut.System1`, polkit `CheckAuthorization`): sürekli çalışan kök
   süreç, etkinleştirme dosyaları, daha geniş saldırı yüzeyi; tek seferlik işler için ağır.
2. **`pkexec` + tek seferlik ikili** (GNOME `gnome-control-center` yardımcıları, `timedatectl`
   olmayan dağıtım araçları gibi): çağrı başına yetki, süreç işini bitirip çıkar, girdi yalnız argv.
3. Kurulumda sudo betikleri: Hoş geldin'den kullanılamaz, denetimi zor.

## Karar

**Seçenek 2.** `/usr/lib/nuxut/nuxut-system-helper` (kökün, 0755), `pkexec` ile; **alan başına ayrı
polkit eylemi** `org.nuxut.system-helper.<alan>` — `exec.path` + `exec.argv1` eşlemesi (polkit 0.113+),
`auth_admin` (keep yok). İlk alan `boot-theme`; greeter `greeter` alanını ekler.

İlkeler (her alan uyar):
1. Girdi yalnız sınırlı argümanlar (sayı, uzunluk, ASCII, denetim karakteri yok); dosya yolu ya da
   içerik kabul edilmez. Yardımcı durumu **kendi** algılar; kullanıcı tarafının planına güvenmez.
2. Yazma yalnız beyaz listeye (`policy.rs`); yedek/geçici adlar izinli yolun kardeşi, sabit kalıpla.
3. Dosya erişimi doğrulanmış dizin tanıtıcılarıyla (`safefs.rs`): bileşen bileşen `O_NOFOLLOW`,
   her dizin kökün ve grup/diğerlerine yazılamaz; atomik yazma; bağın üstüne yazma yok.
4. Her değişiklik yedekli ve günlüklü (bildirim alan başına, `/var/lib/nuxut/<alan>/`); hata =
   o hedefin işlem öncesi hâline dönüş (`ops::Tx`); geri alma yalnız bildirimdekine dokunur.
5. Dış komutlar mutlak yol, temiz ortam, süre sınırı; çıktıları satır satır olay olarak.
6. Çıktı dil bağımsız JSON olaylar (stdout); metni kullanıcı tarafı çevirir.
7. `unsafe` yok, panik yok (`unwrap` yok, `panic = "abort"`); testte gerçek ikili geçici kökle
   (`test-root` özelliği; sürüm derlemesinde `compile_error`) uçtan uca sınanır.
8. Kullanıcı tarafı, ikili ve dizini kökün değilse ya da yazılabilirse pkexec'i çağırmaz.

## Sonuçlar

- Kurulum root ister: paket (üründe) ya da kaynak kurulumunda yönetici onayıyla (aşağıdaki ek; ilk
  sürüm sudo ile kuruyordu). `~/.local` altına kurulamaz — kullanıcının yazabildiği bir ikiliye
  kök yetki verilmez.
- Her uygulama ayrı parola ister (önyükleyiciye dokunan iş için doğru varsayılan).
- Greeter ve sonraki sistem işleri aynı ikiliye alan ekler; yeni bir kök bileşen yazılmaz.

### Ek: en az ayrıcalıkla kurulum (karar 2026-10-17)

- Normal kurulum (`install.sh`) kök dosyalarına dokunmaz ve eksik paket yoksa sudo istemez.
  Yardımcı, polkit eylemi ve giriş ekranı dosyaları kullanıcının dizinine, kurulacakları yolla
  hazırlanır (`~/.local/lib/nuxut/system/usr/…`); bu hazırlık kök yetkisi almaz.
- Kök dosya sistemine yalnız sistem görünümü ya da giriş ekranı istendiğinde ve bir düğmeyle
  kurulur (Hoş geldin, Ayarlar "Güncelle", `install.sh --system-theme`):
  `nuxut-shell system-theme install-helper` hazırlıktaki **sabit listeyi** (`system_bundle::FILES`)
  okuyup sahibi ve izni açıkça yazılmış bir ustar arşivi kurar ve `pkexec /usr/bin/bsdtar` ile `/`
  altına açar (tek onay, Nuxut polkit penceresi). Yardımcı yoksa ya da hazırlıktakinden eskiyse
  `apply` bunu kendiliğinden önce yapar. Kaynak kurulumunda kök yardımcının içeriği zaten
  kullanıcının derlediğidir (sudo ile kurmakla aynı güven); kalıcı olarak kullanıcının yazabildiği
  bir ikiliye kök yetki verilmez ilkesi değişmez (pkexec yalnız kökün `/usr/bin/bsdtar`'ını ve
  kurulmuş `/usr/lib/nuxut/nuxut-system-helper`'ı çalıştırır).
- Ürün kullanıcıya elle `sudo`/`chown`/`pkexec` komutu yazdırmaz (kullanıcı kararı 2026-10-17);
  bekçi testi `tests/root_command_guard.rs`.
