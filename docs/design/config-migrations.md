# Ayar göçleri, kendini onarma ve güncellemede ayarların korunması

Durum: uygulandı (2026-10-16). Kod: `src/config_migrate.rs` (sürüm göçü, ADR-0002),
`crates/nuxut-config-heal` (saf onarım çekirdeği, Ayarlar da kullanır), `src/config_heal/` (shell
kuralları, yedek, bildirim, geri al, durum), `src/config.rs` (`ShellConfig::load_full`), `install.sh`
(kurulum/güncelleme ayrımı, `--reset-config`).

## Karar ve gerekçe (kullanıcı isteği 2026-10-16)

Güncelleme kullanıcının ayarlarını **korur**; ilk kurulum ile güncelleme ayrıdır. install.sh ayar
dosyalarına dokunmaz; eski biçimi kabuk açılışta kendisi taşır (sürüm göçü) ve onarır (kendini
onarma). Yeni anahtarın varsayılanı koddan gelir, dosyaya yazılmaz; hiçbir işe yaramayan satır
temizlenir. Hoş geldin yalnızca gerçek ilk kurulumda gelir (`setup.toml` yokken; kullanıcı isterse
`nuxut-settings --welcome`). Taze kurulum yalnız açık istekle: `sh install.sh --reset-config`
(yedekli; ayrıntı ARCHITECTURE "Kurulum").

İlke: onarım **asla sessizce davranış değiştirmez**. Her "temizlik" adımı bir eşitlik kâhiniyle
doğrulanır: dosya değişiklikten önce ve sonra aynı ayarı (`ShellConfig`, doğrulanmış) veriyorsa adım
uygulanır, vermiyorsa uygulanmaz. Davranışı değiştiren adımlar yalnızca şunlardır ve adlandırılmıştır:
okunamayan satır → yorum, geçersiz değer → varsayılan (eski değer yorumda), eski ad → yeni ad.

## Göç sözleşmesi (geliştirici için)

Şema değişikliği yapan her commit şunlardan birini yapar; `config_schema_snapshot` testi eksiği yakalar:

| Değişiklik | Ne yapılır | Kullanıcı dosyası |
|---|---|---|
| Yeni anahtar | şema anlık görüntüsü güncellenir (`NUXUT_UPDATE_CONFIG_SCHEMA=1`), `since` = sürüm | dokunulmaz; varsayılan koddan |
| Anahtar kaldırıldı | `config_heal::RETIRED`'a eklenir (liste yalnız büyür) | açılışta satır silinir (yedekli, Geri al) |
| Ad değişti (değer aynı anlam) | `config_heal::RENAMES`'e eklenir | açılışta yeni ada taşınır, yorum korunur |
| Tür/birim/anlam değişti | `CURRENT_CONFIG_VERSION` + 1, `config_migrate::MIGRATIONS`'a saf, koşullu adım (`EditKind`), yeni `docs/config-schema/shell-v<N>.txt` | açılışta bir kez göç; davranış değiştiyse sessiz bildirim + Geri al |
| Aralık daraldı | göç gerekmez | aralık dışı değer kullanılan sınıra yazılır (eski satır yorumda) |

Her göç adımının testi: altın eski dosya → altın beklenen dosya (yorumlar ve sıra korunur),
idempotentlik (ikinci çalıştırma değişiklik yapmaz), `EditKind`, çıktı uyarısız ayrışır. Zincir
testi: `tests/fixtures/config/*.toml` (gerçek eski belgeler) → güncel, uyarısız, ikinci açılış hiçbir
şey yazmaz.

Dosyalar: shell.toml tam (göç + onarım); framework.toml yalnızca sözdizimi onarımı (şeması
framework'te; anlamsal onarım için framework'ün bir denetim işlevi gerekir); `displays.toml`,
`setup.toml`, `palette.toml`, `shell-state.toml` makine durum dosyalarıdır, kendi biçim sürümleri var.

## Komutlar

| Komut | Ne yapar |
|---|---|
| `nuxut-shell msg config status` | JSON: dosya sürümü / güncel sürüm, bekleyen göç ve onarım (kuru çalıştırma), yazım önerileri, geri al kaydı, yedek sayısı ve en yenisi, koru listesi. Salt okunur, her örnekte |
| `nuxut-shell msg config undo` | son değişikliği geri alır: önce son onarım (kabuk yeniden başlamış olsa da), yoksa bu oturumdaki sürüm göçü |
| `msg config heal [--dry-run\|--undo\|--fix-typos\|--ignore-typos]` | onar ve yaz / yalnız listele / onarımı geri al / önerileri uygula / yok say |
| `msg config undo-migration` | yalnızca sürüm göçünün geri alı (ADR-0002 §3.5) |

Geri al kaydı `$XDG_STATE_HOME/nuxut/config-heal-undo.toml`'da da durur (0600; iki tam metin,
4 MiB sınırı; bozuksa yok sayılır): güncellemeden sonra kabuk yeniden başlasa da `config undo`
çalışır. Kayıttaki yol yalnızca gerçek ayar dosyasının yoluysa kullanılır.

## Ne zaman çalışır

| An | Ne yapılır | Diske |
|---|---|---|
| Açılış (asıl örnek) | sözdizimi + göç + tam onarım | yazılır: önce yedek, sonra tek atomik yazım |
| `msg config heal` (bildirimdeki "Onar") | aynı | yazılır |
| Dosya değişti (`config reload`, izleyici) | aynı, **yalnızca bellekte** | yazılmaz; değişiklik varsa "Onar" bildirimi |
| İkincil/test örneği | yalnızca bellekte | hiçbir zaman |

Dosya değişince diske yazılmamasının nedeni: dosyayı az önce bir insan ya da araç yazdı. Açık bir
düzenleyicinin altından satır silmek, kullanıcının o an yazdığı yarım anahtarı yok eder ve
düzenleyicide "dosya diskte değişti" çatışması yaratır. Bellekteki onarım yine de çalışan ayarı
korur: tek bir geçersiz değer artık bütün dosyayı düşürmez (önceden tür hatası tüm config'i
varsayılana döndürüyordu).

## Adımlar (sırayla)

1. **Sözdizimi** (`syntax.rs`; framework.toml için de tek adım budur):
   - Tekrar eden anahtar: **son yazılan geçerli** (Hyprland ve çoğu ayar dosyasındaki alışkanlık:
     kullanıcı alta yeni satır ekler). Önceki satır yoruma alınır.
   - Tekrar eden tablo başlığı: ikinci bölümün satırları birinciye taşınır, ikinci başlık kalkar;
     çakışan anahtar varsa yine "son yazılan geçerli".
   - Bozuk başlık (`[bar`, `[bar] x`): onarılabiliyorsa onarılır; onarılamıyorsa başlık **bölümüyle
     birlikte** yoruma alınır (yoksa altındaki anahtarlar bir önceki tabloya düşerdi).
   - Kapanmamış çok satırlı değer: değerin başladığı satırdan hatanın görüldüğü satıra kadar.
   - UTF-8 olmayan bayt: `U+FFFD` olur (yedekte asıl bayt).
   - Diğer: hatalı satır yoruma alınır. Döngü satır sayısıyla sınırlı; panik yok.
2. **Sürüm göçü** (ADR-0002, `config_migrate.rs`): eski `config_version`'lı dosya taşınır.
3. **Geçersiz değer** (tür hatası): satır yoruma alınır, varsayılan geçerli. Yükleyicinin yok sayıp
   varsayılanı kullandığı değer de (ör. bilinmeyen seçenek) aynı biçimde (şema varsayılanından farklı
   ama silinince ayar değişmiyorsa).
4. **Bilinmeyen anahtar** (serde'nin tanımadığı):
   - `RENAMES`'te varsa yeni adına taşınır (yeni ad zaten yazılıysa eskisi etkisizdi: silinir);
   - `RETIRED`'da (kaldırılmış) ise silinir;
   - bilinen bir kardeş ada benziyorsa (Damerau-Levenshtein; büyük/küçük harf ve `-`/`_` farkı
     eşit; kısa adda 1, uzunda 2 düzenleme; eşit yakın iki aday varsa öneri yok; iki harfli ad
     tahmin edilmez) **dokunulmaz**, öneri bildirimi: "bar.siez → size · Düzelt · Yok say";
   - hiçbiri değilse etkisizdir: silinir.
5. **Sınır dışı sayı**: yükleyicinin zaten kullandığı sınır değeri yazılır (davranış aynı kalmıyorsa
   dokunulmaz), eski satır yorumda.
6. **Varsayılana eşit satır**: silinir (Ayarlar zaten böyle yazıyor; dosya yalnızca kullanıcının
   farklarını tutar). Girdide yorumu olan satır (üstünde ya da sonunda) **silinmez**: elle yazılmış ve
   açıklanmıştır.
7. Onarımın boşalttığı tablo başlığı (altında ne değer ne yorum kalan) kalkar.

Daha yeni bir Nuxut sürümünün dosyası (`config_version` büyük): yalnızca sözdizimi; anahtarlarını
tanımadığımız için hiçbir şey silinmez.

Biçim korunur: işlemler metin düzeyindedir (span'lı `toml_edit` ayrıştırması), dokunulmayan her bayt
aynı kalır. Bir anahtar ancak kendi satırına sahipse silinir/yoruma alınır; satır içi tablonun üyesi
gibi paylaşılan satıra dokunulmaz (geçersizse dıştaki satır yoruma alınır).

## Yorumlar: kaldırılan anahtarın üstündeki yorum ne olur?

**Kalır.** Anahtar satırı silinir, hemen üstündeki yorum yerinde durur; satır sonu yorumu
(`size = 30 # benim notum`) kendi satırına taşınır. Gerekçe: yorumlar kullanıcının yazdığı metindir,
çalışan ayarı etkilemez, kullanıcı da "yorumlar kalabilir" dedi (2026-10-16). Onarım bir yorumun
kime ait olduğunu bilemez (bölüm başlığı mı, satırın açıklaması mı, kişisel not mu); silinen not
geri getirilemezken artık bir yorum zararsızdır. Onarımın kendi yazdığı yorumlar biçimlidir:
`# nuxut: geçersiz değer, varsayılan kullanılıyor (2026-10-16)` ve altında eski satır `# …`; bunlar
sonraki onarımlarda yorum olarak kalır (idempotent).

## Yedek, bildirim, geri al

- Her yazımdan önce tam dosya yedeği: `$XDG_STATE_HOME/nuxut/backups/config/<dosya>.<UTC damga>`
  (çakışırsa `.2`, `.3` …; var olan dosyanın üzerine asla yazılmaz; 0600, dizin 0700; dosya başına
  son 10 yedek). Yedek alınamazsa onarım yazılmaz (bellekte kalır).
- Tek sessiz bildirim (popup yok, merkezde okunmamış): "Ayarlarında 3 eski satır temizlendi · Geri
  al" ya da "Ayarlarında 3 satır düzeltildi" + gövdede sınıflar. Yalnızca varsayılana eşit satır
  silindiyse bildirim yok. Aynı sorun kümesi için bekleyen-onarım bildirimi yinelenmez.
- **Geri al** (`msg config undo` ya da `msg config heal --undo`): dosya hâlâ onarımın yazdığı metindeyse onarım öncesi metin
  yazılır; değiştiyse reddedilir ve yedeğin yolu söylenir. Onarımın sildiği/taşıdığı anahtarlar
  **koru listesine** girer (`$XDG_STATE_HOME/nuxut/config-heal.toml`, `[keep] "shell.toml" = [...]`):
  sonraki açılışlar onlara dokunmaz. Bozuk satır ve geçersiz değer korunamaz (dosya okunamaz ya da
  ayar çalışmaz); bunlar sonraki açılışta yine onarılır. Aynı yüklemede sürüm göçü de yazıldıysa
  geri al göçmüş metne döner (göçün kendi geri alı ayrı: `config undo-migration`).
- Komutlar: `msg config heal` (onar ve yaz), `--dry-run` (yalnızca listele; her örnekte),
  `--undo`, `--fix-typos`, `--ignore-typos` (önerileri koru listesine yazar).

## Şema bekçisi (göç gerektiren değişiklik yakalanır)

`docs/config-schema/shell-v<config_version>.txt`: `ShellConfig`'in `Deserialize`'ından çıkarılan
şemanın (yol + tür; enum varyantları dahil) anlık görüntüsü ve kaldırılmış anahtarların kaydı
(`retired`, `renamed <yeni>`). `config_schema_snapshot` testi:

- yeni anahtar → anlık görüntü güncellenmeli (`NUXUT_UPDATE_CONFIG_SCHEMA=1 cargo test
  config_schema_snapshot`); göç gerekmez (varsayılan koddan);
- kaldırılan anahtar → `RETIRED`'a ya da (adı değiştiyse) `RENAMES`'e eklenmeli, yoksa test kırılır;
  bu listeler yalnızca büyür;
- tür değişikliği (enum'a varyant eklemek dışında) → `config_version` artırılmalı ve `MIGRATIONS`'a
  dönüştürme adımı eklenmeli; yeni sürümün dosyası oluşturulurken önceki sürüm dosyasıyla
  karşılaştırılır.

Şema çıkarımı (`trace.rs`): sahte bir `Deserializer` türü "her alan var" gibi sürer ve serde'nin
verdiği alan adlarını kaydeder; her alan ayrı çalıştırmada izlenir (özel `Deserialize`'ı hata veren
alan diğerlerini engellemez). Haritalar `*` parçasıyla; yazım hatası adayları da buradan gelir.

## Ayarlar ve diğer dosyalar

- **Ayarlar** aynı dosyaları yazar ve zaten varsayılana eşit değeri siler, sembolik bağı korur.
  Ortak mantık `crates/nuxut-config-heal`'dedir: bozuk dosyayı açtığında aynı sözdizimi onarımını
  (`repair_syntax`) ve yedek adlandırmasını (`backup`) kullanır. Kırıcı değişiklik yok: crate
  eklemelidir, shell.toml sözleşmesi değişmedi.
- **framework.toml**: yalnızca sözdizimi onarımı (açılışta, asıl örnek, yedekli). Anahtarları
  framework'ün; bilinmeyen anahtar/geçersiz değer onarımı için framework'ün bir denetim işlevi
  (ayrıştırma + bilinmeyen anahtarlar + hata konumu) yayımlaması gerekir (framework talebi).
- **displays.toml, setup.toml, palette.toml, shell-state.toml**: makinenin yazdığı durum dosyaları;
  kendi biçim sürümleri ve okuyucuları var, onarılmaz.

## Bilinen sınırlar

- Widget tabloları (`[bar.widget.<ad>]`) serbest tablodur (kayıt üzerinden çözülür): içlerindeki
  bilinmeyen anahtar serde düzeyinde görülmez, onarılmaz (widget kaydı uyarı verir).
- Dizi tablolarının (`[[...]]`) tek öğesi hedeflenmez; içlerinde silme yapılmaz.
- Anlık görüntü yalnızca serde düzeyini kapsar; değer aralığı değişikliği (ör. üst sınır inerse)
  göç olarak işaretlenmez: aralık dışı değer zaten kullanılan sınıra çekilir.
