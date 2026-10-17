# Emoji seçici: başlatıcının emoji kipi (t123)

Durum: v1 uygulandı, 2026-10-13. Kararlar: kapsam, veri kaynağı, arama, UI.
Kısayol, ten rengi, uygulama araması ve doğrudan yazma: karar 2026-10-13, kullanıcı
onayına açık (aşağıda "Kararlar").

## Amaç ve kapsam

Emojiyi adıyla (Türkçe ya da İngilizce) bulup panoya kopyalamak. Ayrı bir pencere ya da yüzey
değil: başlatıcının üçüncü **kipi** (`LauncherMode::Emoji`), pano kipiyle (`docs/design/clipboard.md`)
aynı kalıp: aynı yaprak, arama alanı ve sanal `ResultList`.

v1:

- Taban emoji (1914; Unicode Emoji ≤ 16.0), ten rengi varyantı yok.
- Arama İngilizce + Türkçe CLDR adı ve anahtar sözcükleri; Türkçe büyük/küçük harf ve aksan
  katlaması.
- Sık kullanılanlar (frecency) başta.
- Seçim: panoya kopyala, başlatıcı kapanır.

Kapsam dışı (v1): odaktaki pencereye yazma (enjeksiyon), ten rengi/cinsiyet varyant seçici,
kısa ad (`:smile:`) sözdizimi, kaomoji/sembol listeleri.

## Tetikleme

- `nuxut-shell msg launcher emoji [--output <ad>]`: başlatıcıyı emoji kipinde açar; o kipte
  açıksa kapatır, başka kipte açıksa kipe geçer. `msg launcher clipboard` ile aynı akış: IPC
  komutu genelleştirildi (`Command::LauncherMode { mode, output }`, `App::launcher_mode`), bind ile
  açılınca klavye yaprak kapanana kadar shell'de kalır.
- **Kısayol `SUPER+.`** (eylem `emoji`; Kararlar 1). Windows'un emoji paneli alışkanlığı,
  varsayılan bind'lerde boştu.
- Uygulama kipinde önek karakteri (ör. `:` ile emoji araması) ya da emoji bölümü **yok**
  (Kararlar 3). Gerekçe: `:` ve benzeri karakterler uygulama/hesap aramasında anlamlı olabilir
  (`:` saat, `.` ondalık); ayrı kip ayrıca klavye odağı, boş sorgu listesi ve sık kullanılanlar
  için temiz bir bağlam.

## Veri

- **Biçim:** `src/services/launcher/data/emoji.tsv`, satır başına bir emoji, sekmeyle ayrılmış
  `emoji  grup  en_ad  en_sözcükler  tr_ad  tr_sözcükler` (sözcükler `|` ile). `include_str!` ile
  derlemeye gömülü; çalışma zamanında dosya okunmaz, sisteme bağımlılık yok. Başlık yorumları
  `# ` ile başlar (`#️⃣` satırı yorum değildir; test eder).
- **Kaynak:** Unicode CLDR annotations (`annotations/en.xml`, `annotations/tr.xml`,
  `annotationsDerived`) ve `emoji-test.txt` sırası. Bu veriyi ibus 1.5.34 paketinin
  `/usr/share/ibus/dicts/emoji-en.dict` ve `emoji-tr.dict` dosyalarından **bir kez**
  dönüştürdük (`examples/emoji_import.rs`, Rust; GVariant `av` kayıtları, her kaydın tür imzası
  doğrulanır). ibus bu dosyaları CLDR'den üretir; veri Unicode License v3 ile dağıtılır,
  lisans metni `data/LICENSE-UNICODE`. Derleme ve çalışma ibus'a bağlı değildir. Ağ erişimi
  olmadığından CLDR XML'i doğrudan indirilemedi; içerik aynıdır.
- **Seçim kuralları** (dönüştürücüde):
  - Aynı emojinin nitelik biçimleri (U+FE0F'li/siz, eksik nitelikli ZWJ biçimleri) tek satır;
    tam nitelikli (en çok U+FE0F) biçim panoya gider.
  - Ten rengi varyantları (U+1F3FB..U+1F3FF içeren 2905 dizi) ve tek başına bileşenler atılır.
  - Grupsuz kayıtlar (CLDR'nin emoji olmayan noktalama açıklamaları, 389) atılır.
  - Emoji sürümü **16.0 ile sınırlı:** dağıtımın renkli fontları (Noto Color Emoji 2.051,
    Twemoji 17.0.2 bile U+1FADD'yi içermiyor) daha yenisini kutu (tofu) çizer. Font güncellenince
    `MAX_EMOJI_VERSION` yükseltilip veri yeniden üretilir.
- **Boyut:** 1914 emoji, ~249 KB salt okunur veri (ikili boyuta eklenen budur; kod küçük).
  Yeni crate yok, derleme süresine etkisi ölçülemeyecek kadar küçük (`include_str!`).
  Ayrıştırma ilk kullanımda bir kez (`OnceLock`); `nuxut-launcher-run` thread'i açılışta tetikler,
  UI thread'i beklemez. Ölçüm (`cargo run --example launcher_bench`, dev profili, kendi crate'imiz
  optimize edilmemiş): ilk ayrıştırma ~71 ms, arama tuş başına ort. ~4,6 ms (en kötü ~12 ms);
  sürüm derlemesinde birkaç kat hızlı. Bellek: katlanmış arama metinleri (~250 KB) + tablo.
- **Yeniden üretim:** `cargo run --example emoji_import -- /usr/share/ibus/dicts/emoji-en.dict
  /usr/share/ibus/dicts/emoji-tr.dict src/services/launcher/data/emoji.tsv`.

## Arama

- **Katlama:** sorgu ve veri aynı `fold_char`'dan geçer: Türkçe `I/İ/ı → i`, ardından
  `nucleo-matcher`'ın basit büyük/küçük harf katlaması ve Latin aksan normalleştirmesi (uygulama
  aramasının `Normalization::Smart` ile aynı tablo): ş→s, ğ→g, ü→u, ö→o, ç→c. "KIRMIZI KALP",
  "kirmizi kalp", "kırmızı kalp" aynı sonucu verir.
- **Eşleşme:** her sorgu kelimesi bir sözcüğün **başında** geçmeli ("kal" → "kırmızı kalp",
  "alp" değil). Kademeler: ad tam eşit (1000) > ad sorguyla başlar (700) > tüm kelimeler adda
  (500) > tüm kelimeler ad/anahtar sözcüklerde (300) > alt dizgi (100, kelime ≥ 2 harf).
- **İki dil:** her emoji iki dilde puanlanır; etkin arayüz dilinin (`i18n::current().languages()[0]`;
  Türkçe değilse İngilizce) puanı tam, diğerinin %80'i. Böylece eşit güçte eşleşmede etkin dil
  önde, ama güçlü bir yabancı dil eşleşmesi zayıf bir etkin dil eşleşmesini geçer: Türkçe
  arayüzde "cat" → 🐈 (İngilizce ad tam, 800), "çatal ve bıçak" değil (Türkçe ad öneki, 700).
- **Sık kullanılanlar:** `$XDG_STATE_HOME/nuxut/launcher.toml` içinde `[emoji]` tablosu (uygulama
  kullanımıyla aynı dosya, aynı atomik yazım ve 0600; ≤ 200 kayıt, anahtar yalnız gömülü verideki
  dizi; tablo boşsa yazılmaz, eski dosyalar aynen okunur). Kayıt `nuxut-launcher-run`
  thread'inde (`LauncherService::record_emoji`): UI thread'inde disk G/Ç yok. Frecency puanı en
  çok 150, kademe aralığından küçük: kullanım kademe içinde öne alır, kademe atlatmaz. Boş
  sorgu: sık kullanılanlar, sonra veri sırası (emoji-test grupları).
- Arama metni loglanmaz; `EmojiSearcher` Debug'ı yalnız uzunluk yazar.

## Arayüz

2026-10-14: v1'in satır listesi framework `GlyphGrid`'e geçirildi (framework
main 8faa968: `GlyphGrid`, `Icon::glyph`, `IconName::Emoji`; framework
`docs/design/glyph-grid.md`). Kod: `src/emoji_view.rs` (`EmojiView`), başlatıcı emoji kipinde
liste alanını ona bırakır.

- **Izgara:** kare hücreler, `GlyphColumns::Auto` (genişliğe sığan sütun). Boş sorguda
  bölümler: "Sık kullanılanlar" (kullanım kaydı olanlar, frecency sırası, en çok 24) + dokuz
  Unicode grubu (yerelleştirilmiş başlık, veri sırası); sık kullanılan emoji kendi grubunda da
  durur. Sorguda tek düz küme (puan sırası, en çok 240). Yeni sorguda ve açılışta ilk öğe seçilir.
- **Kullanıcının yeri korunur (inceleme şartı, 2026-10-15):** kök neden, aynı sorguyla
  gelen her yenilemenin (uygulama kataloğu taraması, kullanım güncellemesi, dil değişimi
  `relocalize`, ilk çizimdeki kapsama denetimi) `GlyphGrid::set_sections` ile ızgarayı yeniden
  kurması ve ilk öğeyi seçmesiydi (`set_sections` sözleşmesi gereği seçim ve kaydırma başa döner).
  Artık: katalog değişimi emoji kipini hiç yenilemez; `EmojiView::refresh` aynı sorgu, aynı dil ve
  aynı satırlarda ızgaraya dokunmaz (seçim ve kaydırma tam yerinde); veri ya da dil değiştiyse
  yeniden kurar ve aynı emojiyi yeniden seçer (aynı indeksteyse orada, değilse ilk göründüğü yer,
  kapsamadan düştüyse aynı indekse kırpılır). Yalnız yeni sorgu, panel açılışı ve kip değişimi
  (`forget_position`) başa döner. Yeniden kurulumda kaydırma ofseti ve süren hareket korunur:
  framework b96e3cb `GlyphGrid::replace_sections` (2026-10-16; eski "ofset korunmuyor" sınırı
  kapandı).
- **Ad satırı (ızgaranın altında):** seçili emoji büyük (`Icon::glyph`, `IconLarge`) + "etkin
  dildeki ad · diğer dildeki ad" (aynıysa tek). Seçim yokken kip ikonu `IconName::Emoji`, metin
  boş; liste boşken "Sonuç yok" ızgara alanında.
- **Klavye:** ↑ ↓ ← → PgUp PgDn Home End Enter ızgaraya (WAI-ARIA grid kalıbı, framework);
  Ctrl/Shift'li oklar arama alanında kalır (sözcük atlama/seçim). Harfler, Space, Backspace,
  Esc arama alanına. Tık seçer ve etkinleştirir. Hesap makinesi bu kipte çalışmaz. Kapanınca kip
  unutulur, sonraki açılış uygulamalar.
- **Renkli glif:** framework atlası renkli glifleri RGBA8 ayrı dokuda tutar; fontconfig yedeğiyle
  Noto Color Emoji çizilir (izole sway'de doğrulandı).
- **Font kapsaması (tofu süzmesi):** ilk çizimde kurulu ilk renkli emoji ailesi (Noto Color
  Emoji, Twemoji, JoyPixels, Apple Color Emoji, Segoe UI Emoji) ile her emoji bir kez
  şekillendirilir (`TextSystem::glyphs_in_family`); tek glife inmeyen emoji (font diziyi
  bilmiyor: kutu ya da parçalı ZWJ çizilirdi) gösterilmez. Aile yoksa süzme yok. Bu VM'de Noto
  2.051 ile 1914/1914 kapsanıyor, denetim ~60 ms (dev profili, yalnız ilk açılışta). Test:
  sistem fontuyla kapsama ≥ %99 ve uydurma ZWJ dizisi reddedilir.
- **Emoji 16.0 sınırı ve süzme:** süzme, verideki emojinin kullanıcının fontunda olmamasına karşı
  güvence (eski dağıtım fontu, yalnız Twemoji). Sınırı kaldırmak için yetmez: Emoji 17 kayıtlarının
  ibus sözlüğünde CLDR adı (özellikle Türkçe) yok, yalnız `E17.0 ad` biçiminde İngilizce
  yorum var. Karar: veri 16.0'da kalır; CLDR 17 Türkçe adları gelince sınır yükseltilir, o zaman
  font kapsamayan kullanıcıda süzme kutuyu önler (dönüştürücüde ayrı font denetimi gerekmez).

## Seçim → pano

Enter/tık → `LauncherOutcome::Emoji` → `Action::PickEmoji(&'static str)` → `App::pick_emoji`:
`cx.set_clipboard(Selection::Clipboard, emoji)` (kabuğun mevcut kopyala yolu, metin araması
alanının Ctrl+C'si ile aynı), `record_emoji`, `ClosePanel(Launcher)`. Seçim kabuk yaşadıkça
sunulur (yaprak kapandıktan sonra da; izole sway'de `wl-paste` ile doğrulandı).

Not: kabuğun kendi koyduğu seçim pano geçmişine **girmez** (`own` bayrağı, clipboard.md aşama 2):
emoji seçimleri pano geçmişinde görünmez. Sık kullanılanlar zaten emoji kipinde.

**Odaktaki pencereye yazma (enjeksiyon) v1 dışı:** `zwp_virtual_keyboard`/`wtype` tarzı sentetik
girdi güvenlik ve odak sorunları taşır (yaprak kapanınca odak dönüşü yarışı, klavye düzeni,
kilit ekranı). Kullanıcı Ctrl+V ile yapıştırır. İleride: framework'te ayrı bir "yazdır" yeteneği
ve kullanıcı onayı ile, v2 (Kararlar 4).

## Testler

- `services::launcher::emoji::tests`: veri bütünlüğü (her satır ayrışır, tekil, iki dilde ad,
  ten varyantı yok, tüm gruplar dolu, `#️⃣`/`*️⃣`, tam nitelikli biçim), Türkçe katlama, sözcük başı
  eşleşmesi, tr/en arama ve dil ağırlığı, boş sorgu sırası, kullanımın kademe içinde kalması,
  sorgu kırpma ve Debug gizliliği, hız.
- `usage::tests::emoji_records_validate_prune_and_roundtrip`: geçersiz anahtar, sınır, gidiş-dönüş,
  eski dosya uyumu.
- `emoji_view::tests`: boş sorguda "Sık kullanılanlar" + grup bölümleri, sorguda düz küme, ok/Enter, sonuç yokken kip ikonu, kapsanmayan emojinin gizlenmesi, sistem fontuyla gerçek kapsama, aynı sorguyla yenilemede yerin korunması (`same_query_refresh_keeps_position`).
- `launcher_view::tests::emoji_mode_ignores_catalog_changes`: katalog değişimi yenilemez; kullanım değişimi ve `relocalize` seçimi korur; yeniden açılış başa döner.
- `launcher_view::tests::emoji_mode_searches_and_picks`: kip açılışı, yer tutucu, arama, Enter →
  `LauncherOutcome::Emoji`, kullanım değişince yenileme, hesap yok, kipin unutulması.
- `ipc::tests::panel_commands`: `launcher emoji [--output]`.
- Canlı (izole sway, test örneği): `msg launcher emoji` → "kalp" yaz → Enter → `wl-paste`
  🫀, `launcher.toml` `[emoji."🫀"]`.

## Kararlar

Karar veren: 2026-10-13, kullanıcı onayına açık.

1. **Kısayol:** `SUPER+.` varsayılan (`keybinds.rs` `Emoji = "emoji", "SUPER+period"`,
   `Target::Shell("msg launcher emoji")`; Ayarlar etiketi `shell-keybinds-emoji-label`).
   Kullanıcının kendi Hyprland bind'i aynı tuştaysa Nuxut'unki üretilmez (mevcut çakışma kuralı);
   Ayarlar'dan değiştirilebilir ya da `""` ile kapatılır. Varsayılanların birbiriyle çakışmadığı
   testle denetlenir.
2. **Ten rengi:** v1 yalnız taban emoji. v2: kalıcı tek bir "varsayılan ten rengi" ayarı
   (`[launcher] emoji_skin_tone`; seçim o varyantı kopyalar; ten destekli emoji listesi veriye
   bayrak olarak eklenir, dizi kuralla türetilir) + satırda `→` ile varyant alt sayfası (eylem
   sayfası kalıbı).
3. **Uygulama aramasında emoji yok:** emoji yalnız emoji kipinde; uygulama listesine bölüm ya da
   önek eklenmez.
4. **v1 yalnız pano:** odaktaki pencereye doğrudan yazma v2 (framework yeteneği ve onay modeli
   gerekir).

## Açık sorular

1. **Font sürümü:** Noto Color Emoji Emoji 17'yi kapsayınca sınır 17.0'a yükseltilecek;
   bunu `install.sh`/paket bağımlılığı mı izlesin?
