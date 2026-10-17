# ADR 0009: Nuxut Qt stili — C++ yalnızca Qt stil eklentisi için

- Durum: kabul edildi. C++ istisnası kullanıcı onaylı (2026-10-16).
- İlgili: ADR-0004 (birleşik görünüm; §5.3 Qt), `qt-style/`, `src/services/look/qt.rs`.

## Bağlam

Birleşik görünüm Qt uygulamalarına renkleri kdeglobals (plasma-integration) ya da qt6ct ile
veriyordu; stil `Breeze` (yoksa `Fusion`). Renkler doğru ama biçim Nuxut'un değil: Breeze'in
köşe yarıçapı ve aralıkları sabit, odak fareyle tıklamada da çiziliyor (vurgu çerçevesi). DESIGN:
"klavye odak halkası (`primary` 2px) her kipte kalır; fareyle tıklamada görünmez".

Qt'de çizimi değiştirmenin tek yolu bir `QStyle` eklentisidir: Qt'nin C++ ABI'siyle derlenmiş paylaşımlı
kitaplık (`QStylePlugin`). Rust'tan yazılamaz (cxx-qt/qmetaobject QWidget stil sınıfı türetmeyi
desteklemez; QStyle sanal yöntemlerinin hepsini FFI'dan geçirmek her boyamada çağrı başına yük
ve bakım yüküdür). Proje kuralı "kod Rust" — bu ADR tek istisnayı tanımlar.

## Değerlendirme: neden kendi stilimiz (Kvantum, Darkly yerine)

| | Kvantum | Darkly | Nuxut (QProxyStyle) |
|---|---|---|---|
| Renk kaynağı | Kendi SVG teması + kvconfig; QPalette'i büyük ölçüde yok sayar | kdeglobals (KColorScheme) | QPalette (kdeglobals/qt6ct, ikisini de Nuxut yazar) |
| Köşe/odak/aralık token'ları | SVG'lere gömülü: her token değişiminde SVG üretmek gerekir | darklyrc'de sabit seçenekler; odak davranışı onun | Doğrudan Nuxut token dosyasından, canlı |
| Canlı değişim | Tema yeniden seçilmeli | Uygulama yeniden başlamalı (yarıçap) | `QFileSystemWatcher` + `StyleChange` |
| Paket | `kvantum` (extra) | yalnız AUR | Kaynak bizde (`qt-style/`), install.sh derler |
| KDE uygulamalarıyla tutarlılık | Ayrı motor | Breeze çatalı | Çizimin çoğu Breeze'in kendisi |

ADR-0004 Kvantum'u zaten yönetim dışı bırakmıştı (SVG temaları paleti izlemiyor). Darkly
Breeze'in çatalı ve yalnızca AUR'da; token'larımızı okumaz.

Seçim: **QProxyStyle**. Taban Breeze (kuruluysa; KDE uygulamalarıyla aynı çizim), yoksa Fusion
(QtWidgets'in içinde, her zaman var). Nuxut yalnızca şunları geçersiz kılar: düğme/alan/açılır
kutu/döner kutu dolgusu (token köşe yarıçapı, Material durum katmanları %8/%12), odak (taban
stile odak durumu hiç verilmez; yalnızca `State_KeyboardFocusChange` varken framework halkası:
`focus_color`, 2 px, iç içe yarıçap), yerleşim aralıkları (4 px ızgara: öğeler arası 8, kenar 12).
Renkler QPalette'ten.

## Karar

1. C++ **yalnızca** `qt-style/` altındaki Qt stil eklentisi için. Başka hiçbir bileşen C++ olmaz;
   eklenti IPC, dosya yazımı ya da ağ yapmaz, yalnızca token dosyasını okur.
2. Token'lar shell'den: `$XDG_CONFIG_HOME/nuxut/qt-style.conf` (`look.mode` ayarının yönetilen
   dosyası, `nuxut-apply` ile; snapshot/geri alma görünümün geri kalanıyla aynı). Biçim:
   `version=1`, `radius_small`, `radius_medium`, `focus_ring`, `focus_color=#rrggbb[aa]`,
   `spacing`. Eklenti ≤ 4 KiB okur, her sayıyı sınırlar, bozuk dosyada varsayılana düşer.
3. Qt6 öncelikli; Qt5 aynı kaynaktan (`-DQT_MAJOR=5`). CMake. install.sh `cmake` ve `qt6-base`'i
   resmî depodan kurar (2026-10-16), eklentiyi derleyip `$LIBEXEC/qt<N>/styles/libnuxut-style.so`'ya
   kurar ve kurulum sonunda doğrular; Qt5 eklentisi yalnız `qt5-base` zaten kuruluysa. Derleme
   başarısızsa kurulum durmaz: uyarı + günlük, Qt uygulamaları Breeze/Fusion ile sürer.
4. Seçim: eklenti kuruluysa kdeglobals `[KDE] widgetStyle=Nuxut` (qt6ct yolunda `style=Nuxut`) ve
   oturum ortamına `QT_PLUGIN_PATH=$LIBEXEC/qt6:$LIBEXEC/qt5` (Qt5 ve Qt6 birbirinin eklentisini
   meta veriden tanıyıp yüklemez; ikisi aynı yolda denendi). Eklenti yoksa önceki davranış (Breeze/Fusion).
5. Testler `qt-style/tests/style_test.cpp` (offscreen QPA, kurulan `.so` dosyadan yüklenir):
   token ayrıştırma ve sınırlar, eklenti anahtarı, fareyle odakta görüntü odaksızla piksel-eşit ve
   halka rengi yok, klavye odağında halka var (düğme ve metin alanı), token dosyası atomik
   değişince ölçü canlı güncellenir. Qt6 (Breeze tabanı) ve Qt5 (Fusion tabanı) ikisi de geçer.

## Riskler ve önlemler

- **Her Qt uygulamasının sürecinde çalışır:** bir çökme bütün Qt uygulamalarını düşürür. Önlem:
  küçük yüzey (≈ 400 satır), dış girdi yalnızca sınırlı token dosyası, taban stile her zaman
  geçerli seçenek; `QProxyStyle(nullptr)` asla (masaüstü stil anahtarı `Nuxut` olduğundan kendini
  sonsuz yaratırdı) — Fusion her zaman var. Odak bilgisini gizlemek için seçenek geçici olarak
  yerinde değiştirilir ve kapsam sonunda geri konur (tek thread, çağıranın yığını).
- **ABI:** eklenti, kurulduğu Qt ana sürümüne bağlı; Qt küçük sürüm güncellemeleri (6.x → 6.y)
  ileri uyumlu. Qt'nin daha eski bir sürüme inmesi ya da Qt 7 eklentiyi yükletmez → Qt taban
  stile düşer (çökme değil). Arch'ta Qt güncellemesinden sonra install.sh yeniden derler.
- **Breeze iç çizimi:** Breeze bazı parçaları kendi işlevleriyle çizer (proxy'ye uğramaz);
  bunlar (kaydırıcı oluğu, onay kutusu göstergesi, menüler) Breeze yarıçapıyla kalır. Kabul:
  en görünür denetimler (düğme, alan, açılır/döner kutu, odak) Nuxut'un.
- **Kabul edilemez risk görülmedi.**
