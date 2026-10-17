# Ekran görüntüsü ve ekran kaydı

Onay: 2026-10-12. Testler yalnızca izole sway'de; kullanıcının
ekranından asla yakalama yapılmaz.

## Kapsam

v1: bölge / pencere / çıkış (monitör) görüntüsü, panoya kopyala + dosyaya kaydet, bildirim.
v1.1: ekran kaydı (bölge / çıkış), bar göstergesi + durdur. Sonra: düzenleyici (ok, kutu,
bulanıklaştırma), Nuxut görünümlü ScreenCast paylaşım seçicisi (portal), framework screencopy.

## Neden shell içinde

Print'e basınca seçim katmanı anında açılmalı; shell zaten çalışıyor, GL bağlamı, fontlar,
monitör/çıkış modeli, Hyprland IPC ve bildirim sunucusu elinde. Ayrı ikili her basışta
GL/font başlatma gecikmesi getirir. Yakalama ve kaydetme mantığı UI'dan bağımsız servis
(`src/services/screenshot/`), test edilebilir.

## Komutlar

```text
nuxut-shell msg shot region|window|output|screen [--output <ad>] [--no-save] [--no-copy]
nuxut-shell msg record start [region|output] [--output <ad>] [--audio] | stop | toggle | status
```
- `region`: dondurulmuş kare üstünde sürükle-seç; seçili değilken tıklanan pencere = pencere.
- `window`: odaklı pencere (Hyprland `activewindow` geometrisi), seçim katmanı yok.
- `output`: odaklı (ya da `--output`) çıkış, seçim katmanı yok. `screen`: tüm çıkışlar tek
  görüntüde (çıkış düzenine göre birleştirilmiş).
- Yanıt: tek satır JSON (`{"saved": "<yol>"|null, "copied": bool}`; iptalde `{"cancelled":true}`).

## Akış (region)

1. Her çıkış için `grim -o <çıkış> -t ppm -` (ya da png) → bellekte görüntü (dondurulmuş kare).
   Çıkışlar paralel yakalanır; ölçek = çıkışın gerçek piksel boyutu.
2. Her çıkışta layer-shell `Overlay` yüzeyi (`KeyboardMode::Exclusive` odaklı çıkışta),
   dondurulmuş kareyi çizer, dışarısı karartılır, seçim dikdörtgeni + boyut etiketi, imleç
   artısı. Hyprland `clients` (görünen çalışma alanları) ile pencere sınırları vurgulanır.
3. Tuşlar: Esc iptal, Enter/Space onay, ok tuşları seçimi 1 px (Shift 10 px) kaydırır,
   sürükleme kenarlarından yeniden boyutlandırma. v1: seçim tek çıkış içinde.
4. Onayda kesim dondurulmuş kareden yapılır (ikinci yakalama yok: görülen = kaydedilen,
   katman görüntüye girmez). PNG kodlama arka thread'de.
5. Kaydet: `[screenshot] dir` (varsayılan `$XDG_PICTURES_DIR/Ekran Görüntüleri`, yerelleştirilmiş
   ad i18n'den; XDG_PICTURES_DIR `user-dirs.dirs`'ten), ad `Ekran görüntüsü YYYY-MM-DD HH-MM-SS.png`,
   çakışmada `-2`. `safe_file` kuralları: sembolik bağlantı izlenmez, `O_EXCL`.
6. Kopyala: `wl-copy --type image/png` (stdin). 7. Bildirim: kabuğun kendi bildirim sunucusuna
   iç bildirim, küçük resim + eylemler "Klasörde göster", "Aç" (xdg-open/OpenURI).

## Kayıt (v1.1)

`wf-recorder` alt süreci (`-g` bölge / `-o` çıkış, `-f` dosya `$XDG_VIDEOS_DIR/Ekran Kayıtları`,
`--audio=<varsayılan çıkışın monitörü>` isteğe bağlı). Durdurma: yalnızca kendi çocuğumuza
SIGINT (dosya düzgün kapanır), 5 sn sonra SIGKILL. Bar göstergesi (kırmızı nokta + süre,
tıklayınca durdur) — widget kaydıyla. `gpu-screen-recorder` alternatif
(DEPENDENCIES.md).

## Kısayollar (hypr_config varsayılanları, Ayarlar Kısayollar sayfasında görünür)

| Tuş | Komut |
|---|---|
| Print | `shot region` |
| Shift+Print | `shot output` |
| Alt+Print | `shot window` |
| Super+Shift+R | `record toggle` (v1.1) |

Kullanıcının zincirinde aynı tuşa bağ varsa (Hyprland ikisini de çalıştırır) mevcut çakışma
denetimi uyarır; kullanıcının dosyasına dokunulmaz.

## Yapılandırma (`shell.toml [screenshot]`)

`dir`, `copy = true`, `save = true`, `notify = true`, `record_dir`, `record_audio = false`.

## Bağımlılıklar

Çalışma zamanı: `grim` (zorunlu, ext-image-copy-capture/wlr-screencopy), `wl-clipboard`,
`wf-recorder` (isteğe bağlı). Crate: PNG/PPM için framework `nuxut-image` ne sunuyorsa o; yoksa
`png` (zaten ağaçta mı, worker bakar). Hepsi `docs/DEPENDENCIES.md`'ye.
İleride: framework'te `ext-image-copy-capture-v1` → grim alt süreci kalkar.

## Test

- Birim: dosya adı/çakışma, XDG dizin çözümü, kesim (ölçekli çıkış, sınır kırpma), seçim durum
  makinesi (sürükle, yeniden boyutla, ok tuşları, pencere tıklama), çoklu çıkış birleştirme,
  grim/wl-copy/wf-recorder komut satırı üretimi, IPC ayrıştırma.
- Canlı: YALNIZCA izole sway (`test-compositor.sh start --owner $$`), iki çıkış + ölçek 1.5
  orada; `NUXUT_SHELL_INSTANCE=<işçi>`; `wtype`/sanal işaretçi yalnızca test display'ine;
  çıktı PNG'si test compositor'ının içeriği; kendi yüzeyini kırparak inceleme.

## v1 uygulama notları ve tasarımdan sapmalar

- `window`: Hyprland `j/activewindow` geometrisi → `grim -g` (dondurulmuş kare yok; katman da
  yok). Hyprland yoksa `error: window: Hyprland bağlantısı yok`.
- `--output` yalnızca `region` (klavyenin/ipucunun verildiği çıkış) ve `output` ile; `window`
  ve `screen`'de hata. `--no-save --no-copy` birlikte hata.
- Seçim: yokken tıklama hemen onaylar (pencere, yoksa çıkış); seçim varken dışarı tıklama onu
  kaldırır. Enter seçim yoksa vurgulanan pencereyi, o da yoksa çıkışı onaylar. Sağ tık = iptal.
  Pencere önceliği: tam ekran > yüzen > döşenmiş, küçük alan önce (Hyprland `clients` sırası
  yığın sırası değil). İmleç artısı çizilmez: `CursorShape::Crosshair` (tema imleci).
- `screen` birleştirmesi kendi kodumuz (grim'in tek çağrısı yerine): tuval ölçeği en büyük
  çıkış ölçeği, protokolün 1/120 adımına oturtulur (sway 1280/1.5 → 853 yuvarlaması).
- Bildirim küçük resimsiz (dahili bildirimde görsel alanı yok — açık iş). Düğmeler
  `shot open|reveal <n>` (bellekte son 16 kayıt; yol komut satırına girmez). "Aç" `xdg-open`
  (portal OpenURI `file://` kabul etmiyor, OpenFile fd ister), "Klasörde göster"
  FileManager1.ShowItems, yoksa klasörü açar.
- `[screenshot]` v1'de `dir`, `save`, `copy`, `notify`; `record_dir`/`record_audio` v1.1 ile.
- `msg` yanıtı seçim bitince; seçim 15 sn'yi (IPC yanıt penceresi) aşarsa istemci yanıt
  alamaz, iş yine yapılır (kısayoldan çağrıda önemsiz).
