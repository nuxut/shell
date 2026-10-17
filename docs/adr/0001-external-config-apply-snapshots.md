# ADR-0001 — Dış yapılandırma uygulama ve snapshot katmanı

Tarih: 2026-10-10 · Durum: önerildi (açık noktalar §11; pano t083/t085 ortak temeli)
Numara: shell deposunun kendi ADR dizisi (`docs/adr/`), calendar deposu gibi 0001'den başlar.
Framework kararlarına `framework ADR-00NN` diye atıf yapılır.

## 1. Bağlam

Nuxut kendi dosyaları dışında da yazmaya başladı ve başlayacak:

| Hedef | Bugün | Yakında |
|---|---|---|
| Hyprland ana dosyası (`hyprland.conf`; ileride Lua) | `source` satırı ekleme — `src/services/hypr_config/ensure.rs::append_source_line` (main) | aynı |
| `nuxut.conf` (Nuxut'un ürettiği) | `sync.rs::write_generated` (`hypr_config` c7a354e) | Lua karşılığı |
| GTK3/4 `settings.ini`, `gtk.css` | — | Birleşik görünüm |
| qt5ct/qt6ct `.conf`, `kdeglobals`, Kvantum | — | Birleşik görünüm |
| dconf/gsettings (`org.gnome.desktop.interface`) | — | GTK4/libadwaita, portal `color-scheme` |
| İmleç `~/.icons/default/index.theme` | — | İmleç seçimi |
| systemd `--user` birimi | — | Başlatma kipi |
| `shell.toml` / `framework.toml` | Ayarlar `store::write_atomic` | Sürüm göçleri (ADR-0002) |

Bugün **üç ayrı atomik yazıcı** var ve güvenceleri farklı:

- `ensure.rs::append_source_line` en sağlamı: symlink hedefine yazar, izni korur, `create_new`
  geçici dosya, `fsync`, yazmadan önce boy/mtime değiştiyse vazgeçer, dizini `fsync`'ler,
  yanına `<ad>.nuxut-<stamp>.bak` yedeği bırakır.
- `sync.rs::write_generated`: geçici + `fsync` + `rename`; değişim denetimi ve symlink
  koruması yok, yabancı dosyayı `.bak` ile yedekler.
- Ayarlar `store::write_atomic`: geçici + `fsync` + `rename`, izin korunur; **symlink'in yerine
  düz dosya koyar** (dotfile yöneticileri kırılır), dizin `fsync` yok, yazım anında dış değişim
  denetimi yok (yalnızca izleyiciye güvenir).

`SyncEnv::guard` (c7a354e) ise yazmanın **güvenli olup olmadığını** ortamdan karar veren tek
yer: Hyprland imzası yoksa hiçbir şey; test örneği (`NUXUT_SHELL_INSTANCE`) yalnızca
kullanıcının `~/.config`'i olmayan, mutlak bir `XDG_CONFIG_HOME` altına yazar ve Hyprland'e
komut göndermez.

Kullanıcı ilkesi: Nuxut kullanıcının dosyalarına dokunuyorsa her değişiklik **geri alınabilir**
olmalı, Ayarlar'da ayar başına "önceki hale dön" bulunmalı ve Nuxut yalnızca yönettiği dosya
parçasına dokunmalı.

## 2. Karar (özet)

1. Kendi dosyaları dışına (ve kendi config'lerinin göçlerine) yazan **her şey** tek katmandan
   geçer: **`nuxut-apply`** kütüphanesi. Yazım, snapshot, işlem kaydı, geri yükleme, kilit ve
   ortam kapısı yalnızca burada.
2. Kütüphane **framework'te** yaşar (yeni crate `nuxut-apply`, `nuxut-config`'in yanında).
   Hedef üreticileri sahiplerinde: Hyprland üretici shell'de (`services/hypr_config`),
   tema → GTK/Qt/kdeglobals/Kvantum/imleç eşlemesi framework'te (yalnızca `Theme`'e bağlı).
3. Snapshot **parça (fragment)** düzeyindedir: TOML anahtarı, INI `[bölüm] anahtar`, dconf
   anahtarı, işaretli satır/blok ya da Nuxut'un tümüyle yönettiği dosya. Yalnızca dokunulan
   parçanın öncesi saklanır.
4. Snapshot'lar **mantıksal ayar** adıyla kaydedilir (`look.unified`, `look.cursor`,
   `hypr.source`, `migration.shell` …). Ayarlar "önceki hale dön"ü ayar başına gösterir.
5. Depo `$XDG_STATE_HOME/nuxut/snapshots/` (yoksa `~/.local/state/nuxut/snapshots/`); işlem
   başına bir dizin, sayı + boyut sınırı, ilk (Nuxut öncesi) hâl sabitlenir.
6. Birden çok süreç (shell, Ayarlar, framework göçü yapan herhangi bir uygulama) aynı katmanı
   doğrudan kullanır; eşzamanlılık `flock` ile çözülür. Shell, yalnızca kendisinin
   yapabildiklerini (Hyprland canlı uygulama, açılışta göç bildirimi) ve CLI erişimini
   `nuxut-shell msg` üzerinden sunar.

## 3. Kavramlar

```rust
/// Bir yazım hedefinin dokunulan en küçük birimi.
pub enum Fragment {
    TomlKey   { file: PathBuf, key: KeyPath },              // toml_edit; biçim/yorum korunur
    IniKey    { file: PathBuf, section: String, key: String }, // satır tabanlı, kendi düzenleyicimiz
    Line      { file: PathBuf, marker: String },            // ör. "source = ~/.config/hypr/nuxut.conf"
    Block     { file: PathBuf, id: String },                // "# >>> nuxut:<id>" … "# <<< nuxut:<id>"
    ManagedFile { file: PathBuf },                          // tümü Nuxut'un; başlık satırıyla tanınır
    Dconf     { key: String },                              // "/org/gnome/desktop/interface/gtk-theme"
    UnitState { unit: String },                             // systemd --user enable/disable durumu
}

/// Bir parçanın değeri. `Absent` = anahtar/satır/dosya yoktu (geri yükleme siler).
pub enum FragValue { Absent, Text(String), File { bytes: Vec<u8>, mode: u32 } }

/// Bir uygulama isteği: hangi ayar, hangi parçalar hangi değere.
pub struct Change { pub setting: SettingId, pub label: String, pub source: Source,
                    pub writes: Vec<(Fragment, FragValue)> }
pub enum Source { Settings, Shell, Welcome, Migration, Cli }

pub struct Snapshot { pub id: SnapshotId, pub at: SystemTime, pub setting: SettingId,
                      pub label: String, pub source: Source, pub state: TxState,
                      pub fragments: Vec<SnapEntry> /* parça + before + after */ }
pub enum TxState { Pending, Applied, RolledBack, Interrupted, Restored(SnapshotId) }
```

`SettingId` noktalı, kararlı bir dizgedir (Ayarlar kataloğundaki kimlikle aynı). `SnapshotId`
zaman sıralı: `20261010T120000Z-3fa1` (UTC + 4 onaltılık rastgele).

## 4. Yazım kuralları (tek yazıcı: `nuxut_apply::write_file`)

`ensure.rs::append_source_line` çekirdek alınır ve genelleştirilir:

1. **Ortam kapısı** önce: `Env::guard()` = `SyncEnv::guard`'ın genel hâli (Hyprland'e özgü
   imza denetimi yalnızca Hyprland hedeflerinde). Test örneğinde tüm dosya hedefleri mutlak ve
   `~/.config` olmayan `XDG_CONFIG_HOME` (dconf/systemd için: test örneğinde **hiç**
   yazılmaz, `Skipped(TestInstance)` döner). Kapı `Err` ise hiçbir dosya açılmaz.
2. **Symlink korunur:** yol `canonicalize` edilir, hedefe yazılır. Hedef yazılamıyorsa ya da
   salt okunur bir depodaysa (ör. `/nix/store`, home-manager) `ExternallyManaged` döner — üstüne
   yazılmaz, symlink kırılmaz.
3. **Boyut sınırı:** okunan dosya ≤ 1 MiB (`nuxut_config::MAX_CONFIG_BYTES` ile aynı); daha
   büyüğü `TooLarge`, yazılmaz.
4. **Atomik:** aynı dizinde `.<ad>.nuxut-<pid>-<n>.tmp` `create_new` + `0600` → yaz →
   eski dosyanın izni (`mode & 0o7777`) → `fsync` → **ön koşul denetimi** → `rename` →
   dizin `fsync`. Hata yolunda geçici dosya silinir.
5. **Ön koşul (yazarken değişti mi):** çağıran okuduğu içeriğin özetini verir
   (`Expect { len, mtime_ns, ino, hash }`); `rename`'den hemen önce yeniden `stat` + gerekirse
   içerik özeti; farklıysa `Changed` ile vazgeçilir ve çağıran yeniden okuyup birleştirir
   (Ayarlar'ın `on_disk_changed` akışı). Kalan TOCTOU penceresi `stat` ile `rename` arası
   mikrosaniyelerdir; dış editörlerle tam kilit mümkün değildir, kabul edildi.
6. **Yalnızca yönetilen parça değişir:** dosya içeriği parça düzenleyicisinden geçer
   (TOML: `toml_edit`; INI: satır tabanlı düzenleyici; Line/Block: işaretli satırlar). Diğer
   satırlar bayt bayt korunur (`append_source_line` testindeki ölçüt her adaptörde test edilir).
7. **İçerik aynıysa yazılmaz** (mtime oynamaz, izleyiciler tetiklenmez).
8. Dizin yoksa yalnızca `ManagedFile`/yeni dosya için oluşturulur (`0700` değil, XDG
   varsayılanı `0755`); kullanıcının ana dosyası yoksa (ör. `hyprland.conf`) yaratılmaz.

`ManagedFile` başlığı: `# Nuxut — <ne> · elle düzenleme bir sonraki uygulamada ezilir`
(mevcut `HEADER_PREFIX`). Başlıksız (yabancı) dosya yönetilen yolda bulunursa ilk yazımda
tüm içeriği snapshot'a girer (bugünkü `.bak`'ın yerini alır, §11-3).

## 5. Adaptörler

| Adaptör | Okuma / yazma | Notlar |
|---|---|---|
| TOML | `toml_edit` | shell.toml, framework.toml, Ayarlar ile aynı kurallar |
| INI | kendi ~200 satırlık düzenleyici | GTK `settings.ini`, qt5ct/qt6ct, `kdeglobals` (`[$e]`/`[$i]` ekli anahtarlar ve diğer satırlar dokunulmadan), Kvantum `kvantum.kvconfig`, imleç `index.theme`. Yorum/sıra/boş satır korunur; crate alternatifi (`rust-ini`) yorumları kaybeder |
| Line / Block | işaretli satırlar | Hyprland `source`, `gtk.css` içinde `@import 'nuxut.css';` |
| ManagedFile | bütün dosya | `nuxut.conf`, `gtk-4.0/nuxut.css`, Kvantum tema dizini, `systemd/user/nuxut-shell.service`, `environment.d/nuxut.conf` |
| Dconf | `dconf read/write/reset` alt süreci, sabit argv, shell yok | `dconf read` boş = ayarlanmamış → `Absent` ayrımı kesin; GVariant metni olduğu gibi saklanır/yazılır. D-Bus `ca.desrt.dconf.Writer` doğrudan kullanımı changeset serileştirmesi gerektirir; kazancı yok. `dconf` yoksa hedef `Unavailable` |
| UnitState | `systemctl --user is-enabled/enable/disable` | Birim dosyası ayrıca `ManagedFile`; `daemon-reload` yan etkidir, snapshot'a girmez |

Alt süreçli adaptörler (dconf, systemctl) zaman aşımlı (2 s), çıktı boyutu sınırlı, ortamı
test örneğinde hiç çağrılmaz. `gsettings` yerine `dconf`: şema kurulu olmasa da çalışır ve
"kullanıcı hiç ayarlamamış" ile "varsayılana eşit" ayrımını korur.

## 6. İşlem akışı

```
apply(change):
  guard() ─Err→ Skipped(reason)
  flock($STATE/nuxut/apply.lock, exclusive, en çok 5 s bekle) ─zaman aşımı→ Busy
  her parça: oku → before   (değer before == after olanlar düşer; hepsi düşerse NoOp, snapshot yok)
  manifest yaz: state = Pending, before/after   (fsync)
  parçaları sırayla uygula (dosya başına tek yazım: aynı dosyadaki parçalar birleşir)
    hata → uygulanmış parçaları before'a döndür → state = RolledBack, Err
  state = Applied (fsync) → journal satırı → kilit bırakılır
```

- Açılışta `Pending` kalmış manifest (çökme) `Interrupted` olarak işaretlenir; **otomatik geri
  alınmaz**, Ayarlar'da "Yarım kalan uygulama · Geri al" olarak görünür (§11-4).
- **Türetilmiş yeniden üretim snapshot üretmez:** tema değişince `nuxut.conf`/`nuxut.css`
  yeniden yazılır; bu, kullanıcının kendi değişikliğinin (framework.toml'daki tema anahtarı,
  Ayarlar geçmişinde) yan etkisidir. Snapshot yalnızca (a) yabancı içeriğe ilk dokunuşta,
  (b) kullanıcıya ait bir anahtar/satır değiştiğinde (ör. `settings.ini`'deki
  `gtk-theme-name`), (c) göçlerde alınır. Böylece depo her tema oynamasında dolmaz.
- Ardışık aynı ayar uygulamaları (Ayarlar'da hızlı tıklama) 2 s içinde tek snapshot'ta
  birleşir: ilk `before` korunur, `after` güncellenir.

## 7. Geri yükleme

`restore(id, mode)`:

- Her parça için **üç yollu denetim:** şu anki değer == snapshot'ın `after`'ı ise `before`
  yazılır. Farklıysa kullanıcı (ya da başka bir araç) o parçayı sonradan değiştirmiştir →
  `Conflict(fragment)`; `mode = Force` değilse o parça atlanır, diğerleri uygulanır
  (`Partial`).
- Geri yükleme de bir işlemdir (yeni snapshot, `source` = çağıran, `state = Restored(id)` eski
  kayıtta) → geri yüklemenin kendisi de geri alınabilir.
- "Ayar başına önceki hale dön" = o ayarın en son `Applied` snapshot'ının geri yüklenmesi.
  "Nuxut öncesine dön" = o ayarın sabitlenmiş ilk snapshot'ı (§8) — Ayarlar'da gelişmiş menüde.
- `Absent` geri yüklemesi anahtarı/satırı/bloğu siler; `ManagedFile` için dosyayı siler ve
  snapshot'ta yabancı içerik varsa onu geri koyar.

## 8. Depo ve sınırlar

```
$XDG_STATE_HOME/nuxut/
  apply.lock
  snapshots/
    journal.log                      # satır başına bir işlem özeti (insan okur, yeniden kurulabilir)
    20261010T120000Z-3fa1/
      manifest.toml                  # Snapshot (yukarıdaki yapı), ≤ 64 KiB
      0.blob, 1.blob …               # yalnızca File değerleri (≤ 1 MiB/adet)
```

- Neden state, cache değil: silinirse geri alma kaybolur (XDG: "kalıcı ama taşınabilir
  olmayan durum"). Neden config değil: kullanıcının dotfile deposuna girmemeli.
- Dizin `0700`, dosyalar `0600` (config içerikleri kişisel olabilir).
- **Saklama:** ayar başına son 20 işlem + her parça için **ilk (Nuxut öncesi) değer
  sabitlenir** (silinmez); toplam en çok 500 işlem ve 64 MiB. Sınır aşılınca en eski,
  sabitlenmemiş işlem silinir. Budama her `apply` sonunda, kilit altında.
- Bozuk/okunamayan manifest atlanır ve loglanır (içerik değil, yalnızca kimlik); depo okunamazsa
  `apply` yine çalışır mı? **Hayır** — snapshot alınamıyorsa dış dosyaya yazılmaz
  (`SnapshotUnavailable`), kendi dosyalarımız (shell.toml normal düzenleme) etkilenmez.
- Loglara dosya içeriği ve değer yazılmaz (ARCHITECTURE değişmez 9); yalnızca ayar kimliği,
  parça türü ve sonuç.

## 9. Kim kullanır, nasıl

### 9.1 Neden framework'te bir kütüphane (shell servisi değil)

| Seçenek | Artı | Eksi |
|---|---|---|
| **A. framework crate `nuxut-apply` (seçilen)** | Ayarlar, shell, framework göçleri, takvim aynı kodu kullanır; shell çalışmıyorken de (Ayarlar tek başına) çalışır; "shell'e özgü olmayan yetenek framework'e aittir" kuralı | Çok süreçli → kilit gerekir (`flock`, std `File::lock`, crate yok) |
| B. Shell servisi + IPC | Tek yazıcı, kilit yok | Shell kapalıyken Ayarlar dış ayar yazamaz; framework.toml göçü shell'e bağımlı olur; IPC'ye dosya içerikleri taşınır |
| C. Ayrı daemon | Tek yazıcı, bağımsız | Yeni süreç, D-Bus arayüzü, kurulum yükü; kazancı A'nın kilidi kadar |

Ayarlar'ın `store::write_atomic`'i ve shell'in iki yazıcısı bu crate'in `write_file`'ına
taşınır (Ayarlar'ın symlink kırma hatası da böylece kapanır). Normal shell.toml/framework.toml
düzenlemeleri **snapshot almaz** (Ayarlar'ın oturum içi geri al geçmişi yeterli); yalnızca
yazıcıyı paylaşır. Göçler ve toplu işlemler (ör. "tüm ayarları sıfırla") snapshot alır.

### 9.2 Sahiplik

- **framework (`nuxut-apply`):** `write_file`, parça düzenleyicileri (TOML/INI/Line/Block),
  Dconf/UnitState adaptörleri, `apply`/`restore`/`list`, depo, kilit, budama, `Env::guard`.
  Ayrıca `nuxut-theme` içinde tema → GTK3/4 / qt5ct-qt6ct / kdeglobals / Kvantum / imleç
  **üreticileri** (saf: `Theme` → `Vec<(Fragment, FragValue)>`).
- **shell:** Hyprland üreticisi ve `source` kararı (mevcut `hypr_config`), shell.toml göçleri
  (ADR-0002), açılışta göç bildirimi, IPC yüzeyi (§9.3). `ensure.rs`/`sync.rs`'teki yazım kodu
  kütüphaneye taşınınca silinir; `SyncEnv` kütüphanenin `Env`'ini kullanır.
- **settings:** UI — ayar başına "önceki hale dön", "Bu sürümde yeni", Hoş geldin penceresi;
  dış hedefleri kütüphaneyle doğrudan uygular.

### 9.3 `nuxut-shell msg` sözleşmesi (yeni)

Satır protokolü (`src/ipc.rs`): başarı `ok` ya da tek satır JSON, hata `error: …`.

```text
snapshot list [--setting <id>] [--limit <1..=200>]   → [{"id","at","setting","label","source","state","fragments":n,"conflicts":n}]
snapshot show <id>                                     → {"id",…,"fragments":[{"kind","target","before":"set|absent","after":"set|absent"}]}
snapshot restore <id> [--force] [--setting <id>]       → {"result":"restored|partial|conflict|not_found|busy","conflicts":[…]}
setup status                                           → §10.2 (+ "setup_state", §10.5)
setup <done|skip> <1..=65535>                          → ok | error: …  (§10.5)
```

- Zaman alanı adı `at` (nuxut-apply ile aynı; `time` değil).
- Snapshot ayar kimlikleri (donuk v1, kapalı liste): `look.unified`, `look.mode`, `look.icons`,
  `look.cursor`, `look.fonts`, `look.motion` (2026-10-13 eklendi, ADR-0004 §18), `look.qt_backend` (birleşik görünüm, ADR-0004), `hypr.source`
  (kullanıcının Hyprland ana dosyasındaki `source`/`require` satırı), `migration.shell`
  (shell.toml sürüm göçü), ve Ayarlar'ın yazdığı TOML anahtarları için şema kimliği (noktalı yol,
  ör. `bar.size`, `compositor.sync_look`). **`hypr.<grup>` ve `displays` kimlik değildir:**
  `nuxut.conf`/`nuxut.lua` ve `displays.lua` türetilmiş dosyalardır (snapshot yok; Hyprland grupları
  shell.toml anahtarının geri yüklenmesiyle döner), Ekranlar geri dönüşü kendi onay penceresi ve
  `displays revert` ile yapılır (ADR-0005), `displays.toml` shell'in küme hafızasıdır.
- Değerler (dosya içerikleri) IPC'den **geçmez**; yalnızca türler ve yollar (yol, ev dizini
  `~` ile kısaltılır). Ayarlar ayrıntıyı kütüphaneyle doğrudan okur.
- `snapshot restore` shell'de çalışır çünkü Hyprland parçası geri yüklenince shell canlı
  uygulamayı (keyword/reload) yapar; Ayarlar Hyprland dışı parçaları doğrudan geri yükler.
- Test örneği kapısı: `NUXUT_SHELL_INSTANCE` ile başlatılmış shell kendi geçici
  `XDG_STATE_HOME`'unu kullanır; kullanıcının deposuna dokunmaz.

### 9.4 Hyprland akışı (c7a354e ile uyum)

Ayarlar Hyprland'e doğrudan yazmaz: shell.toml `[compositor]`/`[windows]`/`[keybinds]`
anahtarlarını yazar; shell'in `nuxut-hypr-sync` thread'i `nuxut.conf`'u üretir (türetilmiş,
snapshot yok) ve `source` satırını ekler (`hypr.source` snapshot'ı, yabancı dosyaya dokunuş).
Sonuç `msg hypr status` (c7a354e) ile okunur.

### 9.5 Uygulama

- Ortam: `main`'de bir kez `Env::from_process(instance)` (kilidi alamamış örnekte
  `disabled = true`); setup.toml, göç, Hyprland eşitlemesi, Ekranlar ve `msg snapshot` aynı
  `Env`'i kullanır. Testler `Env::for_test`. Shell'e özgü yapıştırıcı `services::apply`:
  kapalı kimlik listesi (`known_setting`), kendi dosyası (`write_own`: setup.toml, Hyprland
  "ben ekledim" işareti; snapshot yok), türetilmiş dosya (`write_derived`/`remove_derived`:
  yalnızca yoksa ya da Nuxut başlığı taşıyorsa; başlıksız içerik `externally_managed`, eski `.bak`
  yedeklerinin yerine).
- `snapshot list|show|restore` gövdeleri ayrı thread'de (kilit ≤ 5 sn); IPC yanıt penceresi ve
  `msg` istemcisi 15 sn (`ipc::REPLY_TIMEOUT`; framework varsayılanı 2 sn). Shell'e `snapshot`
  gönderen başka istemci (Ayarlar) de en az bu kadar beklemeli.
- `snapshot restore <id> [--force] [--setting <id>]`: `--setting` verilmiş ve snapshot o ayara
  ait değilse hiçbir şey yazılmaz (`{"result":"not_found"}`). Yanıt `RestoreOutcome` JSON'u
  (`restored` için `"id"` = geri yüklemenin kendi snapshot'ı ya da `null`). Bir şey yazıldıysa:
  `hypr.source` → Hyprland yeniden yüklenir ve eşitleme turu durumu yeniler; `migration.shell` →
  göç bildirimi unutulur, shell.toml yeniden yüklenir; şema kimlikleri → shell.toml yeniden yüklenir.
- `hypr.source` işlemleri: satır ekleme (açıklama + `source`/`require`, `Fragment::Line` ×2;
  yabancı `nuxut.conf`/`.lua` varsa onun `ManagedFile` parçası da), satır varken yabancı üretilen
  dosyanın ilk devralınması, ve "Geri al" (`msg hypr undo`: satırlar + üretilen dosya `Absent`,
  kaynak `cli`). **Geri yükleme satırı kaldırırsa `removed_by_user`** olur ("ben ekledim" işareti
  kalır; satır kendiliğinden yeniden eklenmez, devralınmış yabancı dosya yeniden ezilmez);
  `[compositor] ensure_source = false` görülünce işaret silinir, yeniden açılınca eklenir.
  `msg hypr status` ve `SyncStatus` eski `backup` alanı yerine `snapshot` (bu turun işlem kimliği).
- Göç: geri alınabilir değer varsa `migration.shell` snapshot'ı; "önce" değerleri yeni anlamlı
  eski değerlerdir, böylece `config undo-migration` = o snapshot'ın zorla geri yüklenmesi
  (`snapshot restore <id> --force` ile aynı). Snapshot alınamazsa göç snapshot'sız yazılır,
  geri alma bellekteki kayıtla. `setup status` `pending_notice` = göç snapshot kimliği (yoksa
  `"migration.shell:<from>-<to>"`).

## 10. İlk açılış "Hoş geldin" sözleşmesi (UI Ayarlar'ın)

### 10.1 Adımlar ve çağrılar

| Adım | Kullanıcıya | Uygulama | Snapshot ayarı |
|---|---|---|---|
| 1. Birleşik görünüm | "Uygulamalar Nuxut temasını kullansın mı? (önerilen) — mevcut GTK/Qt/ikon/imleç ayarların değişecek, istediğin an geri alabilirsin" + değişecek dosya listesi (`plan`) | Ayarlar: `nuxut_apply::plan(look::unified(&theme))` → onay → `apply` | `look.unified` |
| 2. İkon ve imleç | Liste (yüklü temalar; framework `nuxut-theme` tarar) | `apply(look::icons(name))`, `apply(look::cursor(name, size))` (+ Hyprland `env`/`setcursor` shell.toml üzerinden) | `look.icons`, `look.cursor` |
| 3. Hyprland ortak ayarlar | "Köşe, gölge, bulanıklık, hareket ve kısayollar masaüstüyle ortak olsun" (Hyprland adı geçmez, yorum 1) | Ayarlar shell.toml `[compositor] ensure_source = true`, `sync_* = true` yazar → shell ekler → Ayarlar `msg hypr status` okur | `hypr.source` |
| Bitiş | — | Ayarlar `msg setup done <sürüm>` (kapatınca `msg setup skip <sürüm>`); `setup.toml`'u shell yazar (§10.5) | — |

`plan(change) -> Plan { entries: [{ fragment, foreign: bool, current: Absent|Set }] }`:
yazmadan, değişecek her parçayı ve **Nuxut'a ait olmayan bir değeri ezip ezmeyeceğini**
döndürür. Pencere "Şunlar değişecek: GTK teması (Adwaita-dark), Qt stili (Breeze) …" metnini
bundan kurar. Hiçbir yabancı değer yoksa uyarı satırı gösterilmez.

### 10.2 `msg setup status` yanıtı

```json
{ "first_run": true, "setup_version": null,
  "hypr": { "session": true, "source": "off|present|added|disabled_by_user|removed_by_user|missing_config|unsupported",
            "launch": "exec_once|external|systemd" },
  "migration": { "pending_notice": "20261010T120000Z-3fa1" | null } }
```

### 10.3 `apply` dönüşleri (Rust ve IPC aynı adlar)

`applied(id) | noop | partial(id, failed[]) | skipped(test_instance|no_session|disabled) |
externally_managed(target) | changed (yeniden dene) | busy | too_large(target) |
unavailable(target: dconf yok vb.) | snapshot_unavailable | error(kısa tür)`.
Ayarlar her birini kullanıcı dilinde gösterir; `externally_managed` için "Bu dosyayı başka bir
araç yönetiyor (ör. home-manager); Nuxut dokunmadı".

### 10.4 Kim başlatır

İlk açılış: shell açılışta `setup.toml` yoksa ve test örneği değilse `nuxut-settings --welcome`
başlatır (bir kez; kullanıcı pencereyi kapatırsa Ayarlar `msg setup skip <sürüm>` çağırır ve
shell `state = "skipped"` yazar, Ayarlar'dan yeniden açılabilir; bkz. §10.5).

### 10.5 Uygulama — donuk v1

§10.1 "Bitiş" satırındaki "Ayarlar `setup.toml` yazar" maddesinin **yerine**: setup.toml'u
**yalnızca shell yazar**; Ayarlar sonucu IPC ile bildirir.

- Dosya `$XDG_STATE_HOME/nuxut/setup.toml` (yoksa `~/.local/state`), shell'e özel biçim:
  `state = "done"|"skipped"`, `version = <1..=65535>`, `at = "<RFC 3339 UTC>"`; bilinmeyen
  anahtarlar yok sayılır. Okuma 4 KiB sınırlı, yazım `nuxut_apply::write_file` ile atomik
  (`services::apply::write_own`, snapshot yok), dizin yoksa 0700 oluşturulur.
- `msg setup done <n>` / `msg setup skip <n>` → `ok` | `error: …`. `n` katı: yalnızca rakam,
  baştaki sıfır/işaret yok, 1..=65535. Daha küçük sürüm büyüğü düşürmez, aynı kayıt yeniden
  yazılmaz (ikisi de `ok`); aynı sürümde `skipped` → `done` yazılır; bozuk dosya üzerine yazılır.
- Yazım kapısı `nuxut_apply::Env::guard`: kilidi almış varsayılan örnek yazar;
  `NUXUT_SHELL_INSTANCE`'lı test örneği yalnızca gerçek ev dizini dışındaki mutlak
  `XDG_STATE_HOME`'a yazar, aksi hâlde `error: skipped(test_instance)` (kilitsiz örnek
  `skipped(disabled)`).
- `msg setup status` §10.2'ye ek `"setup_state": "done"|"skipped"|null`:
  ```json
  {"first_run":true,"setup_version":null,"setup_state":null,
   "hypr":{"session":true,"source":"present","launch":"exec_once"},
   "migration":{"pending_notice":null}}
  ```
  `first_run` yalnızca dosya **yoksa** `true`; bozuk dosyada `false` (ve sürüm/durum `null`),
  uyarı loglanır — Hoş geldin her açılışta yeniden açılmasın. `hypr.source` `msg hypr status`'un
  `source`'u olduğu gibidir (§10.2 kümesine ek olarak `pending` (ilk tur bitmedi), `inactive`
  (Hyprland yok/test kapısı), `unknown`, `error`, `undone` gelebilir); `launch` ilk tur
  bitmeden `null`. `hypr.session` geçerli `HYPRLAND_INSTANCE_SIGNATURE`. `pending_notice`:
  geri alınabilir son göçün `migration.shell` snapshot kimliği (snapshot alınamadıysa opak
  etiket `"migration.shell:1-2"`); geri alma `msg config undo-migration` (§9.5).
- Başlatma: açılışta dosya yoksa, `[setup] welcome = true` (şemada), varsayılan örnek ve Hyprland
  oturumuysa `nuxut-settings --welcome` ayrık (`setsid -f`, beklenmez; `NUXUT_*` ortamı
  aktarılmaz) başlatılır; önce shell'in dizininde, sonra PATH'te aranır, yoksa uyarı. Oturum
  başına bir kez (`$XDG_RUNTIME_DIR/nuxut/welcome-<imza>`): shell çöküp yeniden başlarsa
  pencere tekrar açılmaz. Shell başlatma sonrası dosya yazmaz.

## 11. Açık noktalar (karar gerekli)

1. **Kütüphanenin yeri:** A) framework `nuxut-apply` (önerilen) · B) shell'de modül + Ayarlar
   path bağımlılığı · C) ayrı daemon. → onay.
2. **Varsayılan: Birleşik görünüm adımı önceden işaretli mi?** A) evet, önerilen ve geri
   alınabilir (öneri) · B) hayır, kullanıcı seçer.
3. **Görünür `.bak` dosyaları:** A) kalksın, yalnızca snapshot deposu (öneri; config dizini
   kirlenmez, Ayarlar'dan geri alınır) · B) kullanıcının ana dosyalarına (`hyprland.conf`)
   dokunulduğunda bir kez yanına `.bak` de bırak (bugünkü davranış, ek güvence).
4. **Çökmede yarım işlem:** A) işaretle, Ayarlar'da sun (öneri) · B) açılışta otomatik geri al.
5. **Saklama sınırları:** ayar başına 20 / toplam 500 işlem / 64 MiB + ilk değer sabit (öneri).
6. **Qt yolu:** A) qt6ct/qt5ct + `QT_QPA_PLATFORMTHEME=qt6ct` (`environment.d`) (öneri; Arch'ta
   yaygın) · B) Kvantum ağırlıklı · C) yalnızca `kdeglobals` (Plasma tarzı platform tema eklentisi
   varsa). Kurulu olana göre seçim mi, sabit mi?
