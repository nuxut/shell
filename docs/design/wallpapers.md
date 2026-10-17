# Yerleşik duvar kağıtları

Karar: 2026-10-13 — **kullanıcı kararı** (uygulama). Kaynak tasarım: 6 tasarım × koyu/açık SVG ve
maskotlu 3 tasarım. Sözleşme: yerleşik duvar kağıtları v1
(taslak + KARARLAR, 2026-10-13; sapmalar aşağıda "Sözleşmeden sapmalar").

## Kararlar

1. **Ayrı crate `crates/nuxut-wallpapers`** (shell, kilit ekranı ve ayarlar ortak). SVG'ler
   `include_bytes!` ile ikiliye gömülü (`assets/<id>-dark.svg`, `<id>-light.svg`; ~33 KB toplam).
   Güvenilir kaynak (repo içi) olduğundan süreç içinde render serbest; kullanıcı dosyası değildir.
2. **Kimlikler** (İngilizce kebab, kalıcı; `shell.toml`'a yazılır):

   | Kimlik | Kaynak dosya | Tasarım | Tohum koyu / açık |
   |---|---|---|---|
   | `pair-flight` (**varsayılan**) | es-ucus | b5 eş uçuş | `#9ec5ff` / `#3f6fc6` |
   | `wingtip` | kanat-ucu | b1 kanat ucu girdabı | `#9ec5ff` / `#3f6fc6` |
   | `climb` | tirmanis | b3 tırmanış | `#9ec5ff` / `#3f6fc6` |
   | `migration` | goc | D v2 göç | `#9ec5ff` / `#2f5fb8` |
   | `silhouette` | buyuk-siluet | A büyük siluet | `#9ec5ff` / `#3f6fc6` |
   | `aurora` | aurora | C aurora (açık v2) | `#5d8fe8` / `#8fb6f6` |
   | `cloud-rest` | mascot/b | B bulut molası (maskot) | `#9ec5ff` / `#3f6fc6` |
   | `far-glide` | mascot/a | A uzak süzülüş (maskot) | `#9ec5ff` / `#3f6fc6` |
   | `chase` | mascot/c | C kovalamaca (maskot) | `#9ec5ff` / `#3f6fc6` |

   Maskotlu üçlü kullanıcı kararıyla eklendi (2026-10-13); varsayılan
   `pair-flight` kalır. Tohum yöntemi maskotsuzlarla aynı: tasarımcının "vurgu" rengi (README:
   iz `#9EC5FF` / `#3F6FC6`, `es-ucus` ile aynı iz dili), ortalama renk değil. Maskot gövdesi
   `#5D8FE8` aday değildi: iki kipte aynı ve açıkta ana vurgu değil. Dosyalar yalnız `path`,
   `use`, doğrusal gradyan içerir (filtre/raster/maske/yazı yok); resvg 0.48 ile render ve
   `<use>` çözümü testle doğrulandı (`svgs_use_only_supported_features`, `mascots_are_drawn`).

   Görünen adlar crate'in kendi Fluent kataloğunda (`locales/{tr,en}.ftl`, alan `wallpapers`,
   anahtar `wallpapers-<id>`): tüketici `nuxut_wallpapers::i18n_source()`'u kataloğuna ekler
   (shell `main.rs`; ayarlar kendi `i18n::init`'inde). Böylece adlar tek yerde, iki uygulamada aynı.
3. **Sözdizimi**: `nuxut:<id>` tema kipini izler, `nuxut:<id>/dark|light` sabittir. Gerekçe:
   tek alan (`[wallpaper] path`) hem dosyayı hem yerleşiği taşır — ayarlar, monitöre özel
   geçersiz kılma (`[wallpaper.outputs."<ad>"] path`) ve kilit (`lock.toml wallpaper`) yeni anahtar
   gerektirmez; URI benzeri önek yolla çakışmaz (yollar mutlak ya da `~`; adı `nuxut:` ile başlayan
   göreli dosya için `./nuxut:…`). Ayrı `builtin = "…"` / `variant = "…"` anahtarları değerlendirildi:
   iki alan arasında öncelik kuralı ve her geçersiz kılma düzeyinde ikilenme getiriyordu.
   Bilinmeyen kimlik ya da varyant → config uyarısı + `nuxut:pair-flight` (dosyaya yazılmaz).
4. **Varsayılan**: `[wallpaper] path` yazılmamışsa (taze kurulum dahil) `nuxut:pair-flight`; şema
   varsayılanı ve açıklaması (`shell-config-wallpaper-path-describe`) buna göre. Kullanıcının
   yazdığı değere (`""` dahil: "shell duvar kağıdı çizmesin") dokunulmaz, shell config dosyası
   yazmaz. Davranış değişikliği: eskiden anahtarsız config duvar kağıdı çizmiyordu; üretim öncesi
   "güncelleme = taze kurulum" politikasıyla kabul (hyprpaper kullanan `path = ""` yazar). O politika
   2026-10-16'da kalktı (güncelleme ayarları korur, `docs/design/config-migrations.md`); bu davranış
   değişikliği o tarihten önce yayımlandığı için göç gerektirmez.
5. **Render** (`render`): `resvg` 0.48 (framework ile aynı), çıkışın **tam fiziksel boyutunda**
   (mantıksal × ölçek), cover = tek dönüşüm (ölçek `max(w/2560, h/1440)` + ortalama ötelemesi;
   fazladan piksel çizilmez). `nuxut_image::render_svg` kullanılmadı: güvenilmeyen girdi içindir ve
   `Cover`'da kırpmadan kutudan büyük tuval üretir.
6. **Bant kırıcı** (`deband`): render 8 bit; aurora'nın geniş ışık alanlarında 1 LSB basamaklar
   halka olarak görünür (tasarım notu riski). Eşikli kutu bulanıklığı (yarıçap `h/72`, 4..=32) +
   8×8 Bayer sıralı dither: kutu ortalaması özgünden kanal başına ≤ 1,5 LSB uzaksa piksel yumuşak
   bölgedir ve ortalama dither'la 8 bite iner; aksi hâlde (kuş kenarı, iz) dokunulmaz. Hiçbir piksel
   2 LSB'den fazla değişmez, düz alanlar birebir kalır. Bellek O(genişlik × yarıçap), süre O(piksel).
7. **Bellek içi kare** (karar 2026-10-13; framework 7375116
   `Wallpaper::set_image`): render sonucu `Frame { key, width, height, rgba: Arc<[u8]> }` bileşene
   `set_image(key, frame)` ile kopyasız verilir (`frame` özelliği: `From<Frame> for
   nuxut_image::WallpaperFrame`; ayarlar açmaz). Anahtar `image_key` = FNV-1a 64(SVG ‖ kimlik ‖
   `PIPELINE`); boyut anahtarda değil (framework kare boyutuyla birlikte karşılaştırır, aynısı
   no-op). İlk uygulamadaki disk önbelleği (`$XDG_CACHE_HOME/nuxut/wallpapers/builtin/*.png`,
   yalıtılmış çözücüden geri okuma) **kaldırıldı**: isabette bile 4K'da ~0,5 sn yalıtılmış PNG
   çözümü ödüyordu, soğukta buna PNG kodlama eklenirdi; bellek içi render 1080p'de ~30–100 ms,
   4K'da çoğu ~130–300 ms (en kötü aurora koyu ~0,9 sn; aşağıda ölçüm). Kip değişiminde yeniden
   render edilir (önbellek yok; değişim seyrek). Eski önbellek dizini bırakılır (yeni sürüm ona
   dokunmaz; elle silinebilir — bilinen sınır).
8. **Thread**: süreç başına tek `Renderer` (shell `Shell::wallpaper_builtins`, kilit
   `LockImages::builtins`); iş varken tek thread, kuyruk boşalınca biter (boşta thread yok); aynı
   (kimlik, varyant, boyut) istekleri birleşir. Yüzey başına `Slot` kuşakla eski sonuçları atar.
9. **Tema kipi**: varyant yazılmamışsa çözülmüş `Theme::mode` (framework `[colors] mode`, `auto`
   çözülmüş). `theme_changed` → kip değiştiyse yeni varyant istenir, gelince bileşen çapraz geçişle
   geçer. Kilit ekranı da `set_theme`'de aynısını yapar.
10. **Kilit ekranı**: kaynak sırası `lock.toml wallpaper` → shell.toml genel `[wallpaper] path`
    (masaüstüyle aynı kural: dosya/anahtar yok ya da bozuk → yerleşik varsayılan, `""` → yok) →
    `palette.toml [meta] image` → düz yüzey. Üç kaynağın üçü de aynı ayrıştırıcıdan geçer
    (`WallpaperSource::parse`): palet görseli `nuxut:<id>` olabilir (Ayarlar yerleşik seçince
    yazar; sözleşme §4), bilinmeyen kimlik → varsayılan + log uyarısı. Monitöre özel shell geçersiz kılmaları kilitte yok
    sayılır (tek kaynak, bilinen sınır). Kilit kendi süreci olduğundan kareyi kendisi render eder
    (paylaşılan önbellek yok; madde 7).
11. **Material You tohumu**: render'dan çıkarılmaz (ortalama renk tasarımda bilinçli soluk);
    `Builtin::seed(variant)` sabit tablosu (yukarıda). Shell'de palet çıkarımı yok (palet ayarlarda
    üretilir: `settings/src/services/palette.rs`), bu yüzden API olarak bırakıldı ve
    `msg wallpaper builtins` ile de verilir.
12. **`palette.toml [meta] image` okuyucuları** (kod araması, 2026-10-13): shell okumaz
    (`services/look` yalnız `[meta] mode` vb.), framework `ThemeLoader` (`nuxut-theme`) yalnız
    `mode`'u okur — ikisi de `nuxut:` adresinden etkilenmez. Kilit okur → madde 10 (çözülür).
    Framework `nuxut-palette` bu alanı **yazar** (`generated_palette(palette, image: Option<&Path>)`)
    ve şablonlara `{{image}}` olarak dosya yolu diye verir: Ayarlar `nuxut:` adresini oraya
    geçirirse şablonlar (hyprlock vb. dış araçlar) açılamayan bir "yol" alır → framework talebi
    (aşağıda).
13. **Renk temaları** (kullanıcı isteği 2026-10-16; framework ff2d790, `docs/design/color-themes.md`):
    tasarımlar Nuxut mavisiyle çizildi. `framework.toml [colors] source = "static"` ve `theme =
    neutral|sage|rose|amber|graphite` iken yerleşik, render'dan önce SVG'nin renk öznitelikleri
    temaya taşınarak çizilir: `recolor_svg(svg, |c| tema.recolor(&BLUE, c))` (`Builtin::svg_for`,
    `render_themed`, `render_preview_themed`) — ton aynen kalır (kipin parlaklığı korunur), hue/kroma
    temanın paletine geçer. Tema `Theme::color_theme.unwrap_or(Blue)` (`color_theme_for`): renkler
    duvar kâğıdından geliyorsa (tema yok) tasarımdaki mavi. Nuxut mavisinde baytlar aynıdır
    (yeniden boyama yok). `image_key` mavi dışında tema adını da katar (mavide önceki anahtar aynı);
    `Renderer`/`Slot` anahtarı temayı içerir. Shell `WallpaperSurface::set_theme` ve kilit
    `OutputView::set_theme` tema değişince yeni kareyi ister, kip değişimiyle aynı çapraz geçişle
    gelir. **Kullanıcı görselleri boyanmaz.** Test: `every_theme_recolors_every_svg` (her yerleşik ×
    varyant × tema ayrışır, ortalama parlaklık mavidekinden ≤ 12 uzak, öğe sayısı aynı),
    `themed_frames_differ_and_keys_follow_theme`, `builtin_renders_in_memory_and_follows_theme_mode`.

## Sözleşmeden sapmalar ve uygulama kararları (2026-10-13)

- **Crate yeri `shell/crates/nuxut-wallpapers`**, framework değil (karar 2026-10-13;
  sözleşme KARARLAR 2 "framework'te" diyordu). Ayarlar path bağımlılığıyla bağlanır; ileride
  duvar kağıdı ağ crate'leri (indirme/katalog) de bu dizinde olacak.
- **Adres sözdizimi** sözleşme §1: `nuxut:<id>` (kipi izler) / `nuxut:<id>/<dark|light>` (sabit).
  İlk uygulamadaki `nuxut:<id>:dark` biçimi yayımlanmadı ve **kabul edilmez** (bilinmeyen kimlik
  uyarısı + varsayılan).
- **Kimlikler (9)**: `pair-flight` (varsayılan), `wingtip`, `climb`, `migration`, `silhouette`,
  `aurora`, `cloud-rest`, `far-glide`, `chase`. İlk uygulamanın `swift-pair` / `big-silhouette`
  adları yayımlanmadı; geçiş yok.
- **"path yazılmamış → varsayılan çizilir"**: `[wallpaper] path` anahtarı yoksa (taze kurulum)
  `nuxut:pair-flight` çizilir, dosyaya yazılmadan (madde 4). Sözleşme §5 "palette.toml `[meta]
  image` de yoksa" koşulunu da arıyordu; shell paleti okumadığı için koşul yalnız `path`'e
  bakar. Onay: 2026-10-13.
- **Kip değişiminde yeniden çözüm**: tema yeniden yüklemesiyle aynı an (`theme_changed` →
  `WallpaperSurface::set_theme`; kilit `OutputView::set_theme`), yeni varyant çapraz geçişle
  (sözleşme §6.3 cevabı). `[wallpaper.outputs.<ad>] path` ve `lock.toml wallpaper` aynı adresi
  kabul eder (testler: `wallpaper_builtin_default_and_sources`, `builtin_wallpaper`).
- **index.toml / XDG veri dizini yok** (sözleşme KARARLAR 2): dizin Rust'ta statik
  (`BUILTINS`), `average` alanı yok (Ayarlar önizlemeyi `render_preview` ile birkaç ms'de üretir).

## Ayarlar sözleşmesi

**Seçilen: crate yolu.** Ayarlar `nuxut-wallpapers`'a path bağımlılığıyla bağlanır
(`nuxut-wallpapers = { path = "../shell/crates/nuxut-wallpapers" }`; takvimin
`nuxut-calendar-ipc` emsali) ve şunları kullanır:

- `nuxut_wallpapers::BUILTINS` (sıra = seçicideki sıra), `Builtin::{id, name(), seed(v),
  source(), is_default()}`, `Variant::{Dark, Light}`, `DEFAULT_SOURCE`, `parse_source(&str)`.
- `nuxut_wallpapers::i18n_source()` → ayarların `i18n::init` kaynaklarına eklenir (adlar).
- Önizleme: `render_preview(id, variant, w, h) -> Result<Rendered{width, height, rgba}, RenderError>`
  — opak düz RGBA8, cover, aynı dither. **Bloklar** (önizleme boyutunda birkaç ms; 1080p ~15–80 ms
  sürüm derlemesinde): UI thread'i dışında çağrılır. Gömülü kaynak olduğu için yalıtım gerekmez.
- Seçim yazımı: `shell.toml [wallpaper] path = "nuxut:<id>"` (kipi izler) ya da
  `"nuxut:<id>/dark|light"`; shell config izleyicisi geçişi yapar.
- Palet: yerleşik seçiliyse `source_color_from_file` yerine `Color::from_hex(builtin.seed(v))`.

Gerekçe (IPC `msg wallpaper preview <id> <v> <g>x<y>` → PNG yolu yerine): shell çalışmıyorken de
çalışır; IPC turu, diske önizleme yazımı ve PNG çöz-kodla turu yok; sözleşme derleme anında
denetlenen Rust API'si; ayarlar zaten framework'e path bağımlılığıyla bağlı ve `resvg`/`png`
ağacında var (ek derleme yok). Bedeli: ayarlar ikilisine ~20 KB SVG.

`msg wallpaper builtins` yine de var (tanı, betikler; ad shell'in dilinde):
`[{"id":"pair-flight","source":"nuxut:pair-flight","name":"Eş uçuş","variants":["dark","light"],"default":true,"seed":{"dark":"#9ec5ff","light":"#3f6fc6"}}, …]`.

## Ölçüm (2026-10-13, VM 6 çekirdek, eşzamanlı derlemeler sürerken; sürüm derlemesi)

`cargo run --release -p nuxut-wallpapers --example builtin_bench -- <dizin>`. Render / dither /
PNG kodlama (ms) ve PNG boyutu:

| Çözünürlük | Render | Dither | PNG | Toplam | Dosya |
|---|---|---|---|---|---|
| 1920×1080 | 12–77 (aurora en yavaş) | 19–26 | 11–21 | 44–110 | 1,1–1,7 MB |
| 2560×1440 | 21–133 | 32–39 | 20–55 | 78–217 | 1,9–3,0 MB |
| 3840×2160 | 53–778 (aurora koyu) | 78–163 | 48–125 | 184–1043 | 4,2–6,7 MB |

İlk karar: hepsi > 50 ms → disk önbelleği (isabette yalnızca yalıtılmış PNG çözümü). set_image ile
PNG sütunu ve çözüm turu düştü; önbellek kaldırıldı (madde 7). Geliştirme derlemesinde crate `opt-level = 2` (0'da 1080p ~0,8 sn).

Bant ölçütü (1920×1080, düz bölgelerde 8×8 blok ortalamalarının ikinci farkı, LSB; küçük = halka az):

| Görsel | Önce | Sonra | Değişen kanal | En büyük fark |
|---|---|---|---|---|
| aurora açık | 0,115 | 0,037 | %11 | 2 |
| aurora koyu | 0,141 | 0,046 | %15 | 2 |
| migration koyu | 0,149 | 0,037 | %16 | 2 |
| pair-flight koyu | 0,064 | 0,037 | %8 | 2 |

Görsel karşılaştırma (aurora açık, kontrast ×16): önce belirgin eşmerkezli halkalar, sonra yumuşak
geçiş; yalnız ışık alanının tepe noktasında küçük bir halka kalır (eğrilik eşiği aşıyor; ×1'de
≤ 2 LSB, görünmez).

## Canlı doğrulama (izole sway, `NUXUT_SHELL_INSTANCE=test-wpbuiltin`)

Geçici `XDG_CONFIG_HOME`/`XDG_CACHE_HOME`/`XDG_STATE_HOME`, shell.toml yok: varsayılan eş uçuş
koyu → `[colors] mode = "light"` yazınca açık varyant çapraz geçişle; `scale 1.5` → 2880×1620
render; `nuxut:aurora/light`, `nuxut:migration/dark`; `nuxut:yok` → uyarı + varsayılan; shell
config dosyası yazmadı. Kilit (`NUXUT_LOCK_INSTANCE`, fake-auth): lock.toml/shell.toml duvar
kağıdı yok → açık eş uçuş, sahte şifreyle açıldı.

Maskotlu üçlü (2026-10-13, 1920×1080): `cloud-rest` koyu → kip açık (çapraz geçişle açık
varyant), `far-glide` açık → kip koyu, `chase` koyu, `nuxut:chase/light` (koyu temada açık
varyant sabit). Üçü de iki kipte resvg ile eksiksiz (bulut, iz, maskot yüzleri). Eski
`nuxut:chase:light` → `yerleşik duvar kağıdı "chase:light" bilinmiyor; nuxut:pair-flight` uyarısı.

## Framework talepleri

Üçü de karşılandı (framework, 2026-10-13):
- `Wallpaper::set_image(key, frame)` — 7375116; shell ve kilit kullanıyor (madde 7).
- `render_svg` kırpmalı cover (`TargetSize::cover_crop`) — dd8c253. Crate yine kendi
  `rasterize`'ını kullanır (gömülü kaynak, denetim bütçesi gereksiz; bant kırıcı aynı geçişte).
- `nuxut-palette` yerleşik adresi saydam taşır (`[meta] image`/`{{image}}` = `nuxut:<id>`,
  `{{image_path}}` adreste boş; CLI `generate nuxut:<id> --color #hex`) — 4e1a69b.

## Bilinen sınırlar

- Kilit, shell'in monitöre özel duvar kağıdını değil genel `[wallpaper] path`'i gösterir.
- sway ölçek değişiminde iki configure gönderir: ara boyut da render edilir (bir kez).
- 4K aurora koyu her açılışta ve kip değişiminde ~1 sn render (önbellek yok; o sürede önceki görsel/zemin).
- Ayarlar sözleşmesi uygulanana dek seçici yerleşikleri göstermez; `path`'e elle `nuxut:<id>` yazılabilir.
