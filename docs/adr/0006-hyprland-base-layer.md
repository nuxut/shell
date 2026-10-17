# ADR-0006: Hyprland temel katmanı (sahiplik modelinin genişlemesi)

- Durum: kabul edildi (2026-10-15). Karar sahibi: **kullanıcı, 2026-10-14**:
  "Nuxut Hyprland ayarında tam temel katman sahiplenir: kutudan çıktığı gibi eksiksiz
  bir masaüstü, GNOME/KDE düzeyinde. Kullanıcının hyprland.conf'u en son okunur ve istediğini ezer."
  "Super tuşuna basmak bizim app launcher'ı tetiklemeli."
- İlgili: ADR-0003 (Lua ayarı, sondaki dahil etme satırı, karar 1A), ADR-0001 (dış yazım katmanı),
  `docs/design/hyprland-base.md` (tablolar ve gerekçeler), `docs/research/hyprland-0.56-reference.md`.

## Bağlam

ADR-0003'e kadar Nuxut Hyprland'de yalnızca görünümü (köşe, gölge, bulanıklık, aralık, kenarlık,
hareket), `[input]`'ta yazılanları ve kabuk eylemlerinin kısayollarını üretiyordu; pencere ve
çalışma alanı yönetimi, başlatıcı kısayolu, pencere kuralları kullanıcının dosyasına kalmıştı.
Nuxut'un tek dahil etme satırı dosyanın **sonundaydı** (Nuxut kazanır).

## Kararlar

1. **İki katman.** Yeni temel katman dosyası (`nuxut-base.conf` / `nuxut/base.lua`) kullanıcının ana
   ayar dosyasının **başında** tek satırla dahil edilir; kullanıcının satırları sonra okunur ve onu
   ezer. Ayarlar'da açıkça seçilen değerler (ADR-0003 1A) sondaki dosyada kalır. Gerekçe: tek satır
   hem "kullanıcı ezer" hem "Ayarlar'daki seçim uygulanır" olamaz; varsayılan ile açık tercih farklı
   önceliktedir. **Kullanıcı onayı bekliyor**: kullanıcının sözünü temel katmanla sınırlı yorumladık
   (Ayarlar'ın açık seçimleri hâlâ sonda; ayrı grup anahtarlarıyla kapatılabilir).
2. **Bütün kısayollar temel katmanda** (kabuk eylemleri dahil); kullanıcı `unbind` ile kaldırabilir.
   Aynı tuşu kullanıcı bağlamışsa Nuxut'unki hiç üretilmez (mevcut `split_conflicts`, "aynı fiziksel
   tuş" eşlemesiyle genişledi). Kullanıcının dosyasına satır dışında yazılmaz.
3. **Super'e dokunma başlatıcıyı açar**: `bindr = SUPER, Super_L|Super_R` / Lua `release = true`.
   Araya giren bind/tuş/fare girdisinde Hyprland bind'i gölgeler (kaynakla doğrulandı); bilinen tek
   boşluk belgelendi.
4. **Hareket profili**: masaüstü animasyonları framework token'larından türetilir ama yay/taşma
   yerine Material 3 vurgulu yavaşlama/hızlanma eğrileri ve banda kıstırılmış kısa süreler
   (pencere ≤ 250 ms, alan ≤ 300 ms); pencere stili `popin 92%`. ADR-0003 Bulgu 9'daki "pencereler
   kabukla aynı yayla" eşlemesi kaldırıldı (kullanıcı: "şov için patlamalar değil").
5. **Uygulama açma xdg varsayılanlarıyla** (`msg defaults open <rol>`), sabit uygulama adı yok.
6. **Kapatma anahtarı** `[compositor] base_layer` (vars. açık): kapalıyken temel dosyada yalnızca
   kabuk eylemlerinin kısayolları kalır.
7. **Doğrulama** `Hyprland --verify-config` ile (compositor başlatmadığı kaynakla doğrulandı);
   kullanıcının oturumuna komut gönderilmez.

## Sonuçlar

- Hyprland örnek ayarını aynen kullanan kullanıcıda örnek ayarın bind'leri kazanır; Nuxut'un
  çakışan kısayolları üretilmez (listesi testli). Alt+F4, Super'e dokunma, kilit, ayarlar, oturum
  menüsü yine çalışır.
- Kullanıcının Lua `code:` bind'leri `hyprctl binds`'te görünmez; o bind'le çakışma algılanamaz.
- Eski kurulumlar: ilk açılışta temel katman satırı ayrı bir `hypr.source` işlemiyle eklenir
  (snapshot'lı, "Geri al" ile kalkar).
- Ayarlar → Kısayollar yeni eylemleri listeler (dokunma ve rakam ailesi değiştirici yakalar; fare
  kısayolları yalnızca kaldırılır/varsayılana döner).

## Ek (2026-10-16): kabuk kısayollarının devralınması

Kullanıcı ilkesi "Nuxut yapabiliyorsa varsayılan Nuxut" (2026-10-16) Karar 2'yi kabuk eylemleri
için değiştirir: ses, parlaklık, medya, ekran görüntüsü, emoji gibi Nuxut'un kendi özelliklerinin
kısayolları, kullanıcının ayar katmanından (`nuxut.conf`) ÖNCE okunan aynı tuşlu bind'ini devralır:
`nuxut.conf`'un sonunda `unbind` + Nuxut'un bind'i. Temel katman eylemlerinde kullanıcınınki
kazanmaya devam eder. Kullanıcının dosyasına yazılmaz; `[compositor] take_over_keys = false` geri
alır; `msg hypr status` `taken_over` raporlar. Ayrıntı: `docs/design/hyprland-base.md` §3.4.
