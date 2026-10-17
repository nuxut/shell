# ADR-0002 — Config sürüm geçişi

Tarih: 2026-10-10 · Durum: kabul edildi (§10 hepsi A); shell kısmı uygulandı (§11) · Bağımlı: ADR-0001

## 1. Bağlam

Shell her sürümde anahtar ekliyor, kaldırıyor ve anlam değiştiriyor. Kullanıcı ilkesi:

1. Kullanıcı dosyasında **yalnızca kullanıcının değiştirdikleri** durur; varsayılanlar kodda.
   Yeni anahtar kendiliğinden varsayılanıyla gelir; değişen varsayılan, dokunulmamış anahtara
   kendiliğinden gelir.
2. Kaldırılan anahtar uyarıyla yok sayılır.
3. Yeniden adlandırma / bölme / birim değişimi `config_version` + sıralı, testli göç
   adımlarıyla, açılışta sessizce, önce snapshot (ADR-0001), sıra ve yorumlar korunarak yapılır.
4. Kullanıcının özelleştirdiği listelere (bar widget sırası) güncelleme dokunmaz; yeni widget
   Ayarlar'da "Yeni" rozetiyle önerilir.
5. Göç kullanıcının bir değerine dokunduysa tek sessiz bildirim: "Ayarların yeni sürüme
   taşındı · Geri al" — pop-up yok. Ayarlar'da "Bu sürümde yeni".

## 2. Bugünkü durum (doğrulandı)

**Shell (`src/config.rs`, main f818523):**

- Yükleme `nuxut_config::load` ile: kısmi dosya geçerli, her eksik anahtar `Default`'tan
  (`#[serde(default)]`), **dosya asla yazılmaz** (`nuxut-config` lib.rs ilkesi: okurken yazma +
  izleme = sonsuz döngü). → İlke 1'in okuma tarafı **zaten sağlanıyor**.
- Bilinmeyen anahtar: `nuxut_config` uyarısı, yükleme sürer (`unknown_keys_warn_but_load`).
  Widget tablolarında bilinmeyen anahtar da uyarı (`registry.rs`, `foo = 1` testi).
  → İlke 2 **sağlanıyor**.
- Kaldırılan ama bilinen anahtar: `frame.inner_radius` özel alanla yakalanır, her açılışta
  uyarı + yok sayma (`legacy_inner_radius_is_ignored_with_warning`).
- Yeniden adlandırmalar **çalışma anında** taşınıyor, dosyada değil: `bar.left/right →
  bar.start/end`, `[workspaces]`/`[clock]` → `[bar.widget.*]` (`validate()`); her açılışta
  uyarı verir, dosya eski kalır. `config_version` yok.

**Ayarlar (`../settings`, c2cbfc0; salt okunur incelendi):**

- `store::ConfigFile::set` yalnızca dokunulan anahtarı `toml_edit` ile değiştirir; yorumlar,
  boşluklar, sıra, bilinmeyen anahtarlar korunur. → "Yalnızca değişen anahtar" **sağlanıyor**.
- "Varsayılana dön" düğmesi (`rows.rs` → `write(env, None)`) anahtarı **siler**; boşalan,
  yorumsuz ara tablo da silinir (`remove_key`). → **Sağlanıyor.**
- Seçim satırında varsayılanı seçmek anahtarı siler (`registry.rs::choice_value`). → Sağlanıyor.
- **Açık:** kaydırıcı, anahtar (toggle) ve metin alanında varsayılana eşit bir değer seçilince
  değer **açıkça yazılır** (`Some(default)`), silinmez. Üstelik `show_reset = is_set &&
  !is_default` olduğundan bu durumda sıfırla düğmesi de görünmez. Sonuç: kullanıcı farkında
  olmadan anahtarı eski varsayılana **çiviler**; varsayılan değişince ona ulaşmaz (ilke 1'i
  bozar). → talep §9.2-1.
- Liste düzenleyicisi (`sections.rs`) bir kez dokunulunca listenin tamamını yazar — ilke 4 ile
  uyumlu (listeye dokunan kullanıcı özelleştirmiştir).
- `store::write_atomic` symlink'i düz dosyayla değiştirir ve yazım anında dış değişimi
  denetlemez → ADR-0001 `write_file`'a geçmeli (§9.2-2).

## 3. Karar

### 3.1 Seyrek dosya (ilke 1)

Değişmez: varsayılanlar yalnızca kodda; yükleme dosyayı yazmaz. Ek kural (Ayarlar):
**varsayılana eşit değer yazılmaz, anahtar silinir** — KDE `KConfig`'in davranışı. Bilinçli
olarak "bu değerde sabit kalsın" isteyen kullanıcı dosyayı elle düzenler (açık nokta §10-2).

### 3.2 `config_version`

- shell.toml'un en üstünde tam sayı: `config_version = 2`. Yoksa **1** sayılır (bugünkü tüm
  dosyalar). Kod sabiti `CURRENT_CONFIG_VERSION`; şemada (`nuxut-shell schema`) üst düzey
  `config_version` alanı olarak yayımlanır (şema biçimi değişmez, `SCHEMA_VERSION` 1 kalır —
  Ayarlar bilinmeyen alanları yok sayıyor).
- ~~`config_version` yalnızca **bir göç adımı dosyayı değiştirdiğinde** yazılır.~~ Düzeltme
  (§11, karar): var olan sürümsüz dosyaya adımlar hiçbir şey değiştirmese de shell bir
  kez sürüm damgası yazar; sürümü güncel dosyaya dokunulmaz. ~~Ayarlar yeni dosya yaratırken
  `config_version` yazmaz; adımlar koşullu olduğundan sürümsüz dosya güvenle göç eder.~~
  Düzeltme (§11): v2 adımı sürümsüz dosyada güvenli değil; Ayarlar yeni dosya yaratırken
  `config_version`'ı da yazar (§9.2-4).
- Dosyanın sürümü koddakinden **büyükse** (sürüm düşürme): göç çalışmaz, dosya yazılmaz,
  bilinmeyen anahtarlar bugünkü gibi uyarıyla yok sayılır, tek uyarı: "shell.toml daha yeni
  bir Nuxut sürümüne ait".

### 3.3 Göç adımları

```rust
pub struct Step {
    pub to: u32,                       // bu adımdan sonra dosyanın sürümü
    pub name: &'static str,            // "workspaces-page-count"
    pub run: fn(&mut toml_edit::DocumentMut) -> Vec<Edit>,
}
pub struct Edit { pub key: KeyPath, pub kind: EditKind, pub note: &'static str }
pub enum EditKind {
    Rename { from: KeyPath },          // değer aynen taşınır — sessiz
    DropDead,                          // zaten etkisizdi (ör. ADR-0008 sonrası inner_radius) — sessiz
    Transform,                         // kullanıcının değeri değişti (birim, anlam, sınır) — bildirim
    Drop,                              // etkili bir değer kaldırıldı — bildirim
}
pub const MIGRATIONS: &[Step] = &[ /* to = 2, 3, … sıralı */ ];
```

Kurallar:

- Adımlar **saf** (yalnızca belge; ortam, saat, dosya sistemi yok), **koşullu** (eski anahtar
  varsa ve yenisi yoksa) ve **idempotent** (iki kez çalışmak bir kez çalışmakla aynı).
- `toml_edit` ile: yeniden adlandırmada değerin süslemesi (satır sonu yorumu) ve tablonun
  önündeki yorumlar taşınır; diğer her şey bayt bayt aynı kalır.
- Her adımın testi: (a) altın girdi → altın çıktı metni (yorumlu), (b) idempotentlik,
  (c) `EditKind` beklenen, (d) çıktı `ShellConfig::parse` ile **uyarısız** ayrışır.
  Zincir testi: `tests/fixtures/config/v1-*.toml` → güncel, uyarısız.
- Çalışma anı taşıma kodu (`legacy_left/right`, `legacy_workspaces/clock`,
  `legacy_inner_radius`) ilgili göç adımı eklenince **silinir**: tek doğruluk kaynağı göç.
  `config reload` de aynı yükleme yolundan geçtiği için dışarıdan yapıştırılan eski biçim de göç
  eder.

### 3.4 Çalışma yeri ve sırası

`ShellConfig::load_from(path)`:

```
metni oku (≤ 1 MiB) → sürüm < CURRENT mı? → hayır: bugünkü gibi ayrıştır
evet → toml_edit ile ayrıştır → adımları sırayla belleğe uygula → hiçbir Edit yoksa: yazma, ayrıştır
Edit varsa → nuxut_apply::apply(Change { setting: "migration.shell", source: Migration,
              writes: her Edit'in anahtarı = TomlKey parçası + config_version }) → yeni metni ayrıştır
apply başarısız (busy, changed, snapshot_unavailable…) → bellekteki göçmüş belgeyle çalış,
              dosyaya yazma, bir sonraki açılışta yeniden dene (uyarı loglanır)
```

- Snapshot **anahtar parçası** düzeyinde (ADR-0001 `TomlKey`): Ayarlar "Bu sürümde yeni"
  ekranında hangi anahtarın neye dönüştüğü listelenebilir.
- Döngü yok: göç kendi yazımından sonra izleyici yeniden yüklediğinde sürüm günceldir, adım
  çalışmaz, yazım olmaz. Ayarlar açıkken göç olursa Ayarlar `on_disk_changed → Reloaded`
  akışıyla yeni metni alır (yazım anı denetimi ADR-0001 §4-5 çakışmayı yakalar).
- `nuxut-config`'in "dosya asla otomatik yazılmaz" ilkesine **tek istisna** budur ve
  ilkeye eklenir: "yalnızca sürüm göçü, yalnızca bir şey değiştiğinde, snapshot'la".

### 3.5 Bildirim ve geri al (ilke 5)

- `Rename` ve `DropDead` sessizdir (davranış aynı). Yalnızca `Transform`/`Drop` varsa
  shell **bir** bildirim üretir: başlık "Ayarların yeni sürüme taşındı", gövde değişen ayarların
  kullanıcı adları (en çok 3 + "…"), eylem "Geri al". Pop-up yok: shell'in bildirim sunucusu
  `x-nuxut-silent` ipucunu taşıyan bildirimi doğrudan merkeze (geçmiş) koyar, okunmamış
  sayacını artırır (sunucu tarafında küçük ek).
- **"Geri al" = yalnızca `Transform`/`Drop` anahtarlarının eski değerini yeni anlamıyla geri
  koymak** (ADR-0001 parça bazlı `restore`), `config_version` geri alınmaz. Gerekçe: dosyanın
  tamamını eski sürüme döndürmek bir sonraki açılışta yeniden göçe yol açar (döngü); eski değer
  yeni anlamla doğrulanır (sınır dışıysa uyarıyla sınırlanır). Tüm dosyanın eski hâli yine de
  snapshot'tadır: Ayarlar'ın gelişmiş geçmişinden "göç öncesi dosya" geri yüklenebilir — bu,
  eski Nuxut sürümüne dönen kullanıcı içindir.
- Ayarlar "Bu sürümde yeni": (a) `migration.shell` snapshot'larının anahtar listesi,
  (b) şemada `since > son görülen sürüm` olan ayar ve widget'lar.

### 3.6 Listeler ve "Yeni" rozeti (ilke 4)

- Kullanıcı listeye hiç dokunmadıysa (`bar.end` dosyada yok) varsayılan listedeki yeni widget
  kendiliğinden görünür (ilke 1). Dokunduysa liste olduğu gibi kalır; **göç adımları listelere
  widget eklemez**. Kaldırılan widget adı listede kalırsa bugünkü gibi uyarıyla atlanır; bir
  göç adımı onu `DropDead` olarak listeden çıkarabilir (yeniden adlandırılan widget'ta `Rename`
  öğe düzeyinde).
- Şemaya `since = <config_version>` alanı: `[[widget]]` ve `[[setting]]` öğelerinde,
  yalnızca 2 ve sonrası için yazılır (yoksa 1). Semver yerine `config_version` tamsayısı:
  monoton, dosyada zaten var, karşılaştırması bariz.
- Ayarlar "son görülen sürümü" kendi durumunda tutar (`$XDG_STATE_HOME/nuxut/settings.toml`
  `seen_config_version`), **shell.toml'a yazmaz**. Widget ekleme menüsünde ve ayar satırında
  "Yeni" rozeti `since > seen` iken; Ayarlar "Bu sürümde yeni" sayfası açılınca `seen` güncellenir.
- **§3.6 inceltme (2026-10-13):** "Yeni" rozetinin görülmesi sürüm bazlı
  (`since > seen_config_version`) değil, öğe bazlıdır. Ayarlar kendi durum dosyasında
  (`$XDG_STATE_HOME/nuxut/settings.toml`, `[seen]`: `"<shell.toml yolu | widget:<ad>>" = <since>`)
  her öğeyi ayrı tutar. Öğe ≥1 sn ekranda kalınca görüldü sayılır; ilk açılışta mevcut tüm
  `since`'li öğeler görüldü yazılır. shell.toml'a görülme bilgisi yazılmaz; shell'in sözleşmesi
  yalnızca şemadaki `since` alanıdır. Taze kurulum (`install.sh --reset-config`) durum dosyasını da
  sıfırlarsa rozetler görünmez (beklenen). (Ayarlar tarafı settings 52d91a6.)
- **Kendini onarma (2026-10-16):** güncellemeler artık ayarları sıfırlamaz. Güncel sürümlü dosyada
  kalan eski adlar (`RENAMES`), kaldırılmış anahtarlar (`RETIRED`), geçersiz değerler ve bozuk TOML
  her açılışta onarılır; bir anahtarı kaldıran ya da türünü değiştiren şema değişikliği şema
  anlık görüntüsü testiyle yakalanır (tür değişikliği bu ADR'nin göç adımını ister). Ayrıntı:
  `docs/design/config-migrations.md`.

## 4. İlk göç: `config_version = 2`

| Adım | Girdi | Çıktı | Tür |
|---|---|---|---|
| `bar-start-end` | `[bar] left = […]` / `right = […]` | `start` / `end` yerinde (yenisi zaten varsa onun değeri silinir, eski kod da `left`i kullanıyordu) | Rename (+ yeninin değeri DropDead) |
| `widget-tables` | `[workspaces]`, `[clock]` | `[bar.widget.workspaces]`, `[bar.widget.clock]` (hedef varsa eski silinir; eski kod da yok sayıyordu) | Rename (hedef varsa DropDead) |
| `frame-inner-radius` | `[frame] inner_radius` | silinir (ADR-0008'den beri etkisiz) | DropDead |
| `workspaces-page-count` | `[bar.widget.workspaces] show_empty`, `count` | aşağıda | Transform / DropDead |

`workspaces-page-count` (`workspaces` dd5f0a7 ile birlikte iner; `count` artık sayfa boyu,
1..=30, `show_empty` kalktı):

| Eski | Eski davranış | Yeni dosya | Tür |
|---|---|---|---|
| `show_empty = true`, `count = N ≤ 30` | 1..N her zaman görünür | `count = N`, `show_empty` silinir | DropDead (show_empty) — N yuva yine hep görünür |
| `show_empty = true`, `count = N > 30` | 1..N görünür | `count = 30` | Transform (bildirim) |
| `show_empty = true`, `count` yok | 1..10 görünür | `show_empty` silinir (varsayılan 10) | DropDead |
| `show_empty = false`/yok, `count = N` yazılı | `count` etkisizdi (yalnızca dolu olanlar) | `count` **silinir** (varsayılan 10) | Transform (bildirim; geri al → `count = N` sayfa boyu olarak) |
| `show_empty = false` | — | silinir | DropDead |

Son satırın gerekçesi: eski dosyada hiçbir şey göstermeyen bir sayı, yeni anlamda birden
"her zaman N nokta" olurdu; kullanıcının görmediği bir değer davranışı belirlememeli.

## 5. framework.toml

Aynı ilkeler ve aynı mekanizma geçerli, ama dosya framework'ün: şeması, varsayılanları ve göç
adımları `nuxut-theme`'de yaşar; göçü framework.toml'u ilk yükleyen herhangi bir Nuxut
uygulaması (shell, Ayarlar, takvim) çalıştırır — kilit (ADR-0001) ve idempotent adımlar bunu
güvenli kılar. Genel göç çalıştırıcısı (`Step`, `run_migrations(doc, steps, current)`)
`nuxut-config`'e girer; shell de onu kullanır. palette.toml üretilmiş durumdur, göç edilmez,
yeniden üretilir. → talep §9.1.

## 6. Bağımlılık

`toml_edit` shell'in **doğrudan** bağımlılığı değil (Cargo.lock'ta yalnızca `proc-macro-crate`
derleme bağımlılığı olarak, farklı özelliklerle). Göç uygulamasıyla shell'e `toml_edit = {
version = "0.25", default-features = false, features = ["parse", "display"] }` eklenir
(Ayarlar ile aynı sürüm, `toml` 1 ile aynı `toml_parser` ailesi; derleme maliyeti küçük).
`docs/DEPENDENCIES.md`'ye planlı satır olarak yazıldı. Genel çalıştırıcı framework'e girerse
bağımlılık oradan gelir.

## 7. Reddedilen seçenekler

- **Çalışma anı uyumluluk katmanı (bugünkü `legacy_*`) sonsuza dek:** her eski biçim kodda
  kalır, her açılış uyarı verir, dosya hiç düzelmez, Ayarlar eski anahtarı göremez.
- **Göçü yalnızca Ayarlar yapsın:** Ayarlar'ı hiç açmayan kullanıcının dosyası eski kalır;
  shell yine çalışma anı katmanına muhtaç.
- **Dosyanın tamamını `toml` serde ile yeniden yazmak:** yorumlar, sıra ve bilinmeyen anahtarlar
  kaybolur; ilke 3'e aykırı.
- **Semver `since`:** dize karşılaştırması, ön sürüm etiketleri; `config_version` yeterli.

## 8. Uygulama sırası

1. framework: `nuxut-apply` çekirdeği (ADR-0001) + `nuxut-config` göç çalıştırıcısı.
2. shell: `config_version`, `MIGRATIONS` (v2: §4), `load_from` entegrasyonu, `legacy_*` silinir,
   şemada `config_version` + `since`; bildirim (`x-nuxut-silent`).
3. settings: varsayılana eşit → sil; `write_file`'a geçiş; "Yeni" rozeti, "Bu sürümde yeni".

## 9. Talepler

### 9.1 Framework

1. `nuxut-apply` crate'i (ADR-0001 §9.2) — ya da onay verilmezse shell'de yazılıp sonra
   taşınması için onay.
2. `nuxut-config`: genel göç çalıştırıcısı (`Step`, `Edit`, `EditKind`, `run_migrations`) ve
   "dosya asla yazılmaz" ilkesine göç istisnasının eklenmesi.
3. framework.toml için `config_version` ve kendi adımları; `ConfigWatcher` kendi yazımından
   sonraki yeniden yüklemede göç adımlarının boş geçtiğinin testi.

### 9.2 Ayarlar

1. Kaydırıcı/anahtar/metin: varsayılana eşit değer → anahtarı sil (`choice_value` ile aynı
   kural); `show_reset` buna göre sadeleşir.
2. `store::write_atomic` → `nuxut_apply::write_file` (symlink koruması, yazım anı denetimi).
3. "Yeni" rozeti (`since`, `seen_config_version`), "Bu sürümde yeni" sayfası, ayar başına
   "önceki hale dön" (ADR-0001), Hoş geldin penceresi (ADR-0001 §10).
4. Sürümsüz (`config_version` yok) bir shell.toml'a yazarken şemadaki `config_version`'ı da yaz
   (§11 "sürümsüz dosya belirsizliği"); var olan sürüme dokunma.

## 10. Açık noktalar (karar gerekli)

1. **Göç geri alması:** A) yalnızca değeri değişen anahtarlar, yeni anlamla (öneri) ·
   B) dosyanın tamamı eski hâline + o sürümün göçü "reddedildi" diye işaretlenir (eski anahtarlar
   yine de yok sayılır; kullanıcıya pek bir şey kazandırmaz).
2. **Varsayılana eşit değer:** A) her zaman sil (öneri; KDE gibi) · B) Ayarlar'da "bu değeri
   sabitle" seçeneği (gelişmiş) ile çivilenebilsin.
3. **Kaldırılan ama etkisiz anahtar:** A) göçte sil (`DropDead`, öneri) · B) silme, yalnızca
   uyar (dosya kullanıcınındır).
4. **`workspaces.count` yazılı ama `show_empty` kapalı:** A) `count`'u sil + bildirim (öneri) ·
   B) koru — kullanıcı güncellemeden sonra birden N nokta görür.
5. **Yeni widget varsayılan listeye girsin mi** (listeye hiç dokunmamış kullanıcı için kendiliğinden
   görünür)? A) evet, varsayılan liste ürün kararıdır (öneri) · B) hayır, yeni widget'lar yalnızca
   "Yeni" rozetiyle önerilir, varsayılan liste dondurulur.

Karar (2026-10-10): §10'un tamamı **A** — göç geri alması yalnızca değeri değişen
anahtarlar, yeni anlamla; varsayılana eşit değer yazılınca anahtar silinir; etkisiz anahtar göçte
silinir; `workspaces.count` (show_empty kapalıyken) silinir + bildirim; yeni widget varsayılan
listeye girer (liste özelleştirilmediyse).

## 11. Uygulama notları (shell, `src/config_migrate.rs`)

- `CURRENT_CONFIG_VERSION = 2`, `MIGRATIONS` (§4 adımları), `migrate(text) → Migration`
  (`Current` / `Newer` / `Unchanged` / `Migrated` / `Unparsable`). `ShellConfig::load_with(path,
  writer)` ve `ShellConfig::parse` aynı yoldan geçer (`parse` yalnızca bellekte). Çalışma anı
  `legacy_*` katmanı silindi; güncel sürümde eski anahtar bilinmeyen anahtar uyarısıdır.
- **İdempotentlik çalıştırıcı düzeyinde.** `workspaces-page-count` tek başına idempotent olamaz
  (v2'de `count` + `show_empty` yok = v1'de "görünmeyen count" ile aynı metin). Bu yüzden adımlar
  yalnızca sürümü `to`'dan küçük belgeye uygulanır ve göçen belgeye sürüm damgası yazılır; test
  `migrate(migrate(x)) == Current`.
- **Sürümsüz dosya (karar, review düzeltmesi).** §3.2 "Ayarlar yeni dosyaya
  `config_version` yazmaz" kuralı v2'den sonra bozulurdu: sürümsüz bir dosyaya sonradan
  `[bar.widget.workspaces] count = 15` yazılırsa sonraki açılış onu v1 sayıp silerdi. Karar:
  (a) var olan sürümsüz dosyada adımlar hiçbir şey değiştirmese de shell **bir kez**
  `config_version = 2` damgasını yazar (`stamp_version`, etiket `migration.shell.stamp`; baş
  yorumu üstte kalır, bildirim yok); yazılamazsa (salt okunur, `Changed`) uyarı verilir ve dosya
  o oturum boyunca bellekte güncel sayılır (`LoadState::unversioned_is_current`; yeniden
  yüklemede adımlar çalışmaz, damga yeniden denenir). (b) Ayarlar yeni dosya yaratırken
  `config_version` yazar. Testler: `unversioned_file_is_stamped_and_later_v2_
  count_is_kept`, `settings_written_file_passes_unchanged`, `unstamped_file_counts_as_current_in_memory`.
- **Yalnızca asıl örnek yazar.** Config tek-örnek kararından **sonra** yüklenir;
  `WritePolicy::for_instance`: yalnızca kilidi almış, varsayılan adlı (`NUXUT_SHELL_INSTANCE`
  yok) örnek diske yazar. İkinci çağrı yükleme yapmadan çıkar; test örneği ve IPC'siz çalışma
  göçü yalnızca bellekte uygular (kayıt `written = false`, bildirim/geri al yok,
  `config undo-migration` hata döner). Salt okunur dosyaya (sahip yazma biti yok) yazılmaz:
  `PermissionDenied` → bellekte + uyarı.
- Sürüm anahtarı kökün en üstüne yazılır; dosyanın boş satırla ayrılmış baş yorumu üstte kalır.
  Yeniden adlandırma yerinde (sıra, anahtarın önündeki ve satır sonundaki yorum korunur); taşınan
  tablo eski yerinde yeni başlıkla durur.
- Yazım `ConfigWriter` arayüzünün arkasında (`replace(path, expected, new, label, edits)`);
  bugünkü `AtomicFileWriter`: symlink hedefine, izinler korunur, aynı dizinde gizli geçici dosya +
  `fsync` + `rename` + dizin `fsync`, yazımdan önce ve sonra içerik denetimi (`Changed`), 1 MiB
  sınırı. Yazılamazsa göç bellekte uygulanır, uyarı verilir, kayıt `written = false` (bildirim yok,
  sonraki açılış yeniden dener). `nuxut-apply` gelince yalnızca bu arayüzün uygulaması değişir.
  **Uygulandı:** `AtomicFileWriter` kaldırıldı, yerine `ApplyConfigWriter`
  (`nuxut_apply::write_file` + `migration.shell` snapshot'ı; ADR-0001 §9.5).
- Geri al: `MigrationUndo` (bugün süreç belleğinde; yalnızca yazılmış ve `restore`'lu kayıt) +
  `nuxut-shell msg config undo-migration`. `Restore { key, value }` değeri TOML yazımı olarak
  taşır (snapshot deposuna serileştirilebilir). Ara tablo silinmişse örtük yaratılır, tablo
  olmayan bir şeye dönmüşse `Conflict`.
- Bildirim: `MigrationRecord::notice()` (başlık, en çok 3 ayar adı, "Geri al" →
  `config undo-migration`). `Shell::on_config_migrated` bunu dahili sessiz bildirim olarak
  ekler (`InternalNotice::silent`: popup/ses yok, merkezde okunmamış; düğme komutu `ipc::parse`
  ile `msg` yolundan yürür). Yalnızca bellekte kalan göç bildirilmez.
- Şema: kökte `config_version`; `[[widget]]` ve `[[setting]]`'te `since` (2+; `SettingSchema::
  since`). Sürüm 1 = kullanıcının kurulu shell'inin (stable c567983) anahtarları; v2'de gelenler
  `schema.rs`'teki tek tabloda: `WIDGET_SINCE` (launcher widget'ı), `SETTING_SINCE`
  (`bar.edge_actions.*`, `audio.backend`, `launcher.*`, `compositor.*`, `windows.*`,
  `keybinds.*`), `WIDGET_SETTING_SINCE` (workspaces `style`/`scroll`, active_window
  `max_chars`). Test eskimiş girdiyi yakalar.
- İniş sırası: `workspaces-page-count` v2 anlamını (`workspaces`: `count` 1..=30 sayfa boyu,
  `show_empty` yok) varsayar, `workspaces`'ten **sonra** ya da onunla birlikte iner.
- v2 geliştirmesinde gelen tablolar (`[launcher]`, `[compositor]`, `[windows]`, `[keybinds]`,
  workspaces `style`/`scroll`) yalnızca **yeni** anahtardır (v1 dosyada yoktular; ilke 1 ile
  varsayılandan gelir): göç adımı gerekmez (test `new_v2_tables_need_no_step`); "Yeni" rozeti
  şemadaki `since = 2` ile.
- **Yeni widget türleri kullanıcının açık panel/bar listesine göçle eklenmez** (karar,
  2026-10-13; `night_light` kartı ve `calendar` kartı vesilesiyle). `widgets = [...]` yazmış
  kullanıcının listesi onun seçimidir; sessizce kart eklemek yerleşimini değiştirir. Yeni türler
  yalnızca varsayılan listeye girer (yazılmamış listeye ilke 1 ile gelir) ve Ayarlar'ın "Ekle"
  menüsünde `since` "Yeni" rozetiyle görünür. Güncelleme ayarları korur (kullanıcı isteği
  2026-10-16); listeye kart eklemek yine yalnız kullanıcının işidir. Alternatif — liste sonuna ekleyen göç adımı — kullanıcının
  bilerek çıkardığı kartı geri getirirdi (eklenmemiş ile çıkarılmış ayırt edilemez).
