# Tasarım notu: Ayarlar ↔ Hyprland ortaklığı

Durum: taslak (2026-10-10). Kaynak: kullanıcı yorumu 1.

> **Güncelleme:** Hyprland ≥ 0.55 Lua ayarı, `keyword` yerine `eval`, systemd ile başlatma ve "Geri al"
> için `docs/adr/0003-hyprland-lua-config-and-launch.md` esastır; bu notun hyprlang (`nuxut.conf`,
> `source`, `exec-once`) bölümleri geçiş dönemi arka ucunu anlatır. Eşleme tablosu, kısayol modeli ve
> Ayarlar sözleşmesi (§4–§6) aynen geçerlidir.
>
> **Güncelleme (ADR-0006, 2026-10-15):** kısayollar artık `nuxut.conf`'ta değil, kullanıcı dosyasının
> başında dahil edilen temel katmanda (`nuxut-base.conf` / `nuxut/base.lua`); eylem kümesi ve çakışma
> kuralları `docs/design/hyprland-base.md` §3.

> Kullanıcının istediği: Hyprland ayarında Nuxut'a ait dosyayı gösteren tek satır (yoksa
> eklensin); köşe, hareket, gölge, bulanıklık, kısayollar gibi ayarlar Ayarlar'dan değişince hem
> shell'e hem Hyprland'e uygulansın. Arayüzde "Hyprland ayarları" diye bir bölüm **olmayacak**:
> kullanıcı "Köşeler", "Hareket", "Kısayollar" görür; Hyprland'in de bunları izlemesi ayrıntıdır.

## 1. Bugünkü durum

- `install.sh` `~/.config/hypr/nuxut.conf`'u yazar (`exec-once = nuxut-shell` + ses/parlaklık
  bind'leri) ve soruyla `hyprland.conf` sonuna `source = ~/.config/hypr/nuxut.conf` ekler
  (`grep 'hypr/nuxut.conf'` ile denetim; yorum satırı ve source zinciri bilinmez).
- Shell **systemd ile başlatılmıyor**; tek yol `exec-once` (nuxut.conf'tan). Takvim daemon'u
  ayrı (D-Bus etkinleştirme + `nuxut-calendard.service`).
- Shell tek örnektir (`nuxut::ipc::try_single_instance`): ikinci `nuxut-shell` başlamaz. Çift
  `exec-once` bu yüzden zararsız ama gürültülüdür (log, kısa süreç); yine de üretilmez (§3.3).
- Gerçek dünya örneği (geliştirici makinesi): `hyprland.conf` göreli yollarla
  `source=configs/default/*.conf` kullanıyor, eski projeden kalma `nuxutshell.conf` başka bir
  `exec-once = nuxut-shell` taşıyor, `nuxut.conf` hiç yok. Tasarım bu durumu doğru ele almalı:
  source eklenmeden önce `nuxut.conf` üretilmeli ve dışarıdaki `exec-once` görülüp bizimki
  yazılmamalı.

## 2. Dosyalar ve sahiplik

| Dosya | Sahibi | İçerik |
|---|---|---|
| `<hypr>/hyprland.conf` (ve source ettikleri) | kullanıcı | Nuxut yalnızca **bir kez, bir satır** ekleyebilir (§3). Başka hiçbir satır değişmez. |
| `<hypr>/nuxut.conf` | Nuxut (shell üretir) | Tamamı üretilir; başlığında "elle düzenlemeyin". `exec-once` (gerekirse), görünüm anahtarları, hareket, kısayollar. |
| `~/.config/nuxut/framework.toml` | Ayarlar | Tema: köşe, gölge, hareket, renkler (shell + tüm Nuxut uygulamaları). |
| `~/.config/nuxut/shell.toml` | Ayarlar | Shell'e özgü: `[windows]`, `[keybinds]`, `[compositor]` (yeni, §6). |
| `$XDG_STATE_HOME/nuxut/hypr-source.state` | shell | "Source satırını ben ekledim" işareti (§3.4). |

`<hypr>` = Hyprland ayar dosyasının dizini (§3.1). Tek üretilmiş dosya yeter: Hyprland her
`reload`'da tüm zinciri yeniden okur, `exec-once` yalnızca oturum başında çalışır; yeniden
üretim zararsızdır. `install.sh` artık `nuxut.conf`'un içeriğini kendisi yazmaz; (sonraki adım)
`nuxut-shell hypr write` alt komutunu çağırır ya da yalnızca dosya yoksa tohum yazar. Böylece
tek üretici olur.

## 3. (a) Source satırı denetimi ve eklenmesi

### 3.1 Gerçek ayar yolu

Sıra: (1) çalışan Hyprland'in komut satırındaki `-c/--config <yol>`: `hyprctl -j instances`
→ `HYPRLAND_INSTANCE_SIGNATURE`'a uyan örneğin `pid`'i → `/proc/<pid>/cmdline` (saf ayrıştırıcı:
`config_path_from_args`); (2) `$XDG_CONFIG_HOME/hypr/hyprland.conf`; (3)
`$HOME/.config/hypr/hyprland.conf`. Dosya yoksa **oluşturulmaz** (Hyprland ilk açılışta kendi
şablonunu üretir; boş dosya yazmak kullanıcının masaüstünü bozar) → durum `MissingConfig`.
Ana dosya `.conf` değilse (ör. ileride başka biçim) dokunulmaz → `Unsupported`.

### 3.2 Zincir taraması (saf, sınırlı)

`hyprlang` kurallarının alt kümesi, `src/services/hypr_config/parse.rs`:
- `#` yorum başlatır, `##` düz `#`'dir. `# hyprlang if/endif/noerror` yönergeleri yorum sayılır
  (koşullu blokların içi de taranır — muhafazakâr: "sourced" demek için tek görülmesi yeter).
- `$ad = değer` değişken tanımı; değerlerde `$ad` (en uzun eşleşme) yerine konur.
- `source = <yol>`: `~` ve `~/`, `$değişken`, mutlak ya da **source eden dosyanın dizinine
  göre** göreli yol; joker (`*`, `?`, `[...]`) her bileşende, eşleşmeler sıralı (glob(3) gibi).
- Sınırlar: derinlik ≤ 8, dosya ≤ 64, dosya boyu ≤ 1 MiB, satır ≤ 4096 bayt, joker eşleşmesi ≤
  64. Döngü kanonik yolla engellenir. Sınır aşımı `truncated = true` → güvenli tarafta kal:
  satır **eklenmez**, durum bildirilir.
- Rapor (`ChainReport`): taranan dosyalar, hedefi (nuxut.conf, kanonik ya da normalize yol)
  source eden satır var mı, **yorumlanmış** bir `source …nuxut.conf` satırı var mı, nuxut.conf
  dışındaki `exec`/`exec-once` satırlarında `nuxut-shell` başlatılıyor mu (komutun ilk
  sözcüğünün taban adı; `env X=1 nuxut-shell`, `uwsm app -- nuxut-shell` gibi sarmalayıcılar da),
  kullanıcının bind'leri (§5.3) ve yönetilen anahtarları kimin yazdığı (§4.3).

### 3.3 Karar (saf: `plan_source`)

| Koşul | Sonuç |
|---|---|
| `[compositor] ensure_source = false` | `Off` — hiçbir şey yapılmaz |
| ayar dosyası yok / desteklenmiyor | `MissingConfig` / `Unsupported` |
| zincir taraması kesildi | `Unknown` — eklenmez, bildirilir |
| hedef zaten source ediliyor | `Present` |
| yorumlanmış `source …nuxut.conf` var | `DisabledByUser` — eklenmez |
| daha önce biz ekledik (durum dosyası) ve şimdi yok | `RemovedByUser` — tekrar eklenmez, bir kez bildirilir |
| diğer | `Add` |

`exec-once` kararı ayrı (`LaunchMode`): süreç systemd altında (`INVOCATION_ID` ya da
`SYSTEMD_EXEC_PID`) → `Systemd`; zincirde başka yerde `nuxut-shell` başlatılıyor → `External`;
diğer → `ExecOnce` (nuxut.conf `exec-once` içerir). İlk iki durumda nuxut.conf `exec-once`
**yazmaz**: çift başlatma olmaz.

### 3.4 Yazım (yalnızca `Add`)

Sıra: (1) nuxut.conf üretilir (yoksa source satırı Hyprland'de "dosya yok" hatası verir);
(2) ana dosya okunur (≤ 1 MiB); (3) yedek: aynı dizine `hyprland.conf.nuxut-YYYYmmdd-HHMMSS.bak`
(`create_new`, aynı izinler); (4) yeni içerik = eski bayt bayt + (son satır `\n` ile bitmiyorsa
`\n`) + `\n# Nuxut: kabuk ayarları (ekleyen nuxut-shell; kapatmak: shell.toml [compositor]
ensure_source = false)\nsource = <yol>\n`; yol `~/` ile yazılır (ev dizini altındaysa);
(5) atomik yazım: aynı dizinde `O_EXCL` geçici dosya → yaz → `fsync` → izinleri kopyala →
yazmadan hemen önce dosya boyu/mtime değişmediyse `rename` (değiştiyse vazgeç, sonra tekrar
dene); (6) durum dosyasına işaret; (7) log (`info`) + kullanıcıya bildirim ("Hyprland ayarınıza
Nuxut satırı eklendi; yedek: …"). Ana dosya symlink ise (stow/dotfiles) **hedef** dosyaya yazılır;
symlink korunur. Hedef salt okunursa (Nix) hata bildirilir, başka bir şey denenmez.

> **Güncel (ADR-0001 §9.5):** (3)–(5) artık `nuxut-apply`: `.bak` yedeği yerine
> `hypr.source` snapshot'ı (açıklama + `source` satırı `Fragment::Line` ×2; yabancı `nuxut.conf`
> varsa onun da parçası), atomik yazım framework'ün tek yazıcısında. Bildirim yedek yolu yerine
> "Geri al" düğmesi; durum JSON'unda `backup` yerine `snapshot` (işlem kimliği).

Satır **sona** eklenir (öncelik için §4.3). Yeniden yükleme gerekmez: Hyprland ayar dosyasını
izler (`misc:disable_autoreload` kapalıysa kendisi yükler); kapalıysa yeni değerler sonraki
`hyprctl reload`'da ya da canlı uygulamada (§4.4) gelir.

Ne zaman: shell açılışında arka plan thread'inde, yalnızca `HYPRLAND_INSTANCE_SIGNATURE` varsa
(sway/test compositor'ında hiçbir şey yapılmaz). Test/ikincil örnek (`NUXUT_SHELL_INSTANCE`
varsayılan değil) hiçbir zaman yazmaz.

## 4. (b) Ortak ayarlar

### 4.1 İlke

Tek ayar, iki tüketici. Kullanıcının gördüğü ad kullanıcı odaklıdır; değer temada ya da
shell.toml'da durur; shell onu hem kendi çiziminde kullanır hem nuxut.conf'a çevirir. Ayarlar
uygulaması Hyprland'i hiç bilmez (yalnızca durum gösterir, §6.3).

### 4.2 Eşleme tablosu

| Ayarlar'da | Kaynak anahtar | Hyprland karşılığı |
|---|---|---|
| Görünüm → Köşeler | `framework.toml` `shape.radius_large` | `decoration:rounding = round(r)` |
| Görünüm → Gölge | `framework.toml` `shadow.enabled/blur/offset_y/opacity` + `colors.shadow` | `decoration:shadow:enabled`, `range = round(blur)`, `offset = 0 round(offset_y)`, `color = rgba(shadow × opacity)`, `render_power = 3` |
| Görünüm → Hareket (ön ayar, hız, kapalı) | `framework.toml` `[motion]` | `animations:enabled`; `bezier = nuxut_*`; `animation = windows/windowsOut/layers/fade/workspaces` (§4.5) |
| Görünüm → Bulanıklık | `shell.toml` `[windows] blur`, `blur_size` (yeni) | `decoration:blur:enabled/size`, `passes = 2` |
| Pencereler → Aralık | `shell.toml` `[windows] gap_inner`, `gap_outer` (yeni) | `general:gaps_in`, `general:gaps_out` |
| Pencereler → Kenarlık | `shell.toml` `[windows] border` (yeni) | `general:border_size`; `col.active_border = rgba(primary)`, `col.inactive_border = rgba(outline × 0.4)` |
| Kısayollar | `shell.toml` `[keybinds]` (yeni) | `bind…` satırları (§5) |

Renkler temadan (palet, duvar kağıdından türeyebilir) gelir; palet değişince nuxut.conf yeniden
üretilir. `[windows]` Hyprland'e özgü göründüğü hâlde arayüzde "Pencereler" adıyla durur;
pencere kavramı kullanıcının dünyasındadır.

### 4.3 Öncelik: kullanıcının kendi değerleri

Hyprland ayarı sıralıdır: aynı anahtarı **sonra** yazan kazanır. Source satırı sona eklendiği
için Nuxut'un yönettiği anahtarlar kullanıcınınkileri ezer — Ayarlar'da değiştirilen değer
gerçekten uygulanır. Ezilen anahtarlar tarama sırasında bulunur (`managed_overrides`: anahtar,
dosya, satır) ve `hypr status` ile Ayarlar'a verilir ("Bu değer hyprland.conf:12'deki değeri
geçersiz kılıyor"). Kullanıcı bir grubu Nuxut'tan bağımsız tutmak isterse grup anahtarı
kapatılır (`[compositor] sync_look/sync_motion/sync_windows/sync_keybinds`): o grubun satırları
nuxut.conf'a hiç yazılmaz. Source satırını kullanıcı elle yukarı taşırsa (kendi değerleri
kazansın diye) buna saygı gösterilir: satır "var" sayılır, yeri değiştirilmez.

### 4.4 Uygulama (canlı)

Shell `framework.toml`, `palette.toml` ve `shell.toml` değişimini zaten izler. Değişimde:
`generate(input) → (metin, anahtarlar)`; metin öncekinden farklıysa nuxut.conf atomik yazılır,
sonra:
- yalnızca görünüm/hareket/pencere değiştiyse: tek `[[BATCH]]keyword k v;keyword …` (yeniden
  yükleme yok, titreme yok);
- kısayollar değiştiyse: `reload config-only` (bind'lerin silinmesi ancak yeniden okumayla
  temiz olur);
- ardından `configerrors` okunur; nuxut.conf satırına ait hata varsa bildirilir.
Hyprland erişimi `HyprControl` özelliği arkasındadır (`keyword_batch`, `reload`,
`config_errors`); testlerde sahte uygulama komutları kaydeder. Değerler yalnızca üreticiden
gelir; `;`, satır sonu ve denetim karakterleri reddedilir (batch ayırıcısı enjeksiyonu yok).

### 4.5 Hareket eşlemesi

Hyprland yay bilmez; her rol bir bezier + süreye çevrilir (yaklaşık, bilinçli):
- `motion.enabled = false` ya da `instant` → `animations:enabled = false`.
- Rol stili `curve`: eğri `CubicBezier::parse` ile (ad ya da `cubic-bezier(...)`), süre
  `duration_ms / speed`.
- Rol stili `spring`: süre ≈ yayın oturma süresi `response × (1.6 − 0.6·min(damping,1)) /
  speed` (sakin ≈ 380 ms, çevik ≈ 300 ms); eğri az sönümlüde (`damping < 1`) taşmalı
  `(0.3, 1 + o, 0.4, 1)`, `o = exp(−π ζ / √(1−ζ²))` (≤ 0.5), kritikte
  `emphasized-decelerate`.
- Hyprland hızı desisaniye: `max(1, round(ms / 100))`, en fazla 30.
- Roller: `windows`, `layers`, `workspaces` ← `spatial`; `windowsOut` ← `spatial_fast`;
  `fade`, `border` ← `effects`. Bezier adları `nuxut_spatial`, `nuxut_fast`, `nuxut_effects`.

## 5. Kısayollar

### 5.1 Model

Kapalı eylem kümesi (genişletilebilir enum), her eylemin varsayılan tuşu ve Hyprland'deki
karşılığı shell'de tanımlıdır:

| Eylem (`shell.toml` anahtarı) | Varsayılan | Bind |
|---|---|---|
| `volume_up` / `volume_down` | `XF86AudioRaiseVolume` / `…Lower…` | `bindel … exec nuxut-shell msg volume up/down` |
| `volume_mute`, `mic_mute` | `XF86AudioMute`, `XF86AudioMicMute` | `bindl` |
| `brightness_up` / `brightness_down` | `XF86MonBrightnessUp/Down` | `bindel` |
| `media_play`, `media_next`, `media_prev` | `XF86AudioPlay/Next/Prev` | `bindl … exec playerctl …` |
| `panel_right`, `panel_left`, `panel_clock` | atanmamış | `bind … msg panel toggle …` |
| `bar_toggle` | atanmamış | `bind … msg bar toggle` |
| `launcher` | atanmamış (başlatıcı inince) | `bind … msg launcher toggle` |

```toml
[keybinds]
volume_up = "XF86AudioRaiseVolume"
panel_right = "SUPER+N"
bar_toggle = ""          # boş = atanmamış (satır üretilmez)
```

Tuş dizgesi: `MOD+MOD+Tuş`; değiştiriciler `SUPER|SHIFT|CTRL|ALT` (`WIN`, `CONTROL` takma
adları), tuş xkb keysym adı (`[A-Za-z0-9_]`, ≤ 32) ya da `code:<n>`. Diğer her şey geçersiz
(uyarı, satır üretilmez). Exec komutunun mutlak yolu `std::env::current_exe()` (kurulum yolu).
Kullanıcının istediği keyfi komut (`exec firefox`) bu sürümde **yok**: kendi bind'leri
hyprland.conf'ta kalır.

### 5.2 Normalleştirme

`(değiştirici maskesi, tuş küçük harf)`; Hyprland'in `$mainMod` gibi değişkenleri tarama
sırasında çözülür; değiştiriciler `SUPER_SHIFT`, `SUPER SHIFT`, `SUPERSHIFT` biçimlerinde
ayrıştırılır.

### 5.3 Çakışma

- Nuxut içinde aynı tuş iki eyleme atanmışsa ikincisi atılır (uyarı).
- Kullanıcının zincirindeki (nuxut.conf hariç) bir bind aynı tuşu kullanıyorsa Nuxut'un
  bind'i **üretilmez** (Hyprland ikisini birden çalıştırırdı) ve çakışma raporlanır; Ayarlar
  satırın yanında "Bu kısayol Hyprland ayarlarınızda kullanılıyor (dosya:satır)" gösterir.
  Kullanıcının dosyası hiç değişmez. İstisna (2026-10-16): kabuk eylemleri ayar katmanından önce
  okunan bind'i `nuxut.conf`'ta `unbind` ile devralır (`[compositor] take_over_keys`;
  `docs/design/hyprland-base.md` §3.4).

## 6. Arayüz sözleşmesi (Ayarlar uygulaması için)

### 6.1 Anahtarlar

Ayarlar yalnızca TOML yazar (bugünkü mekanizma, atomik). Yeni `shell.toml` anahtarları
(şemada, `nuxut-shell schema` ile; sınırlar):

```toml
[compositor]
ensure_source = true     # açılışta source satırı yoksa ekle (bir kez)
sync_look = true         # köşe + gölge + bulanıklık
sync_motion = true
sync_windows = true      # aralık + kenarlık
sync_keybinds = true

[windows]
gap_inner = 4            # 0..=64 px
gap_outer = 8            # 0..=128 px
border = 2               # 0..=16 px
blur = true
blur_size = 8            # 1..=32

[keybinds]               # eylem = "MOD+Tuş" | ""   (§5.1)
```

`framework.toml` anahtarları değişmez (`shape.radius_large`, `shadow.*`, `motion.*`).
Ayarlar'daki yerleşim: "Görünüm" sayfasında Köşeler/Gölge/Bulanıklık/Hareket; "Pencereler"
grubunda Aralık/Kenarlık; ayrı "Kısayollar" sayfası. "Hyprland" kelimesi yalnızca gelişmiş
bölümdeki "Masaüstü ayar dosyasıyla eşitle" (`[compositor]`) açıklamasında geçer.

### 6.2 Yaşam döngüsü

Ayarlar yazar → shell dosya izleyicisi → yeniden üret + uygula (§4.4). Ayarlar shell'e komut
göndermek zorunda değildir.

### 6.3 IPC (yalnızca durum ve elle tetikleme)

```text
nuxut-shell msg hypr status    → tek satır JSON (aşağıda)
nuxut-shell msg hypr sync      → yeniden üret + uygula (zorla)
nuxut-shell msg hypr add-source→ ensure_source=false iken bile tek seferlik ekle (kullanıcı isteğiyle)
```

```json
{"source":"present|added|off|disabled_by_user|removed_by_user|missing_config|unsupported|unknown|error",
 "config":"~/.config/hypr/hyprland.conf","generated":"~/.config/hypr/nuxut.conf",
 "snapshot":"20261010T120000Z-3fa1",
 "launch":"exec_once|external|systemd",
 "overrides":[{"key":"decoration:rounding","file":"~/.config/hypr/hyprland.conf","line":12}],
 "conflicts":[{"action":"panel_right","combo":"SUPER+N","file":"…","line":40}],
 "errors":["nuxut.conf:14: …"]}
```

Yollar `~/` ile kısaltılır; kullanıcı dosya içeriği (satır metni) JSON'a girmez.

## 7. Uygulama adımları

1. **Yapıldı (eab7732):** `src/services/hypr_config/` — ayrıştırma, zincir taraması,
   `plan_source`, `append_source_line` (yedek + atomik), `config_path_from_args`, üretici,
   bind normalleştirme/çakışma, `HyprControl` + sahte.
2. **Yapıldı:** `[compositor]`, `[windows]`, `[keybinds]` (`hypr_config/settings.rs`; şemada,
   doğrulanmış). Varsayılanlar 1A/2A/3A (sona ekle, `radius_large`, `exec-once`).
3. **Yapıldı:** `app/hypr_sync.rs` → `nuxut-hypr-sync` thread'i (`hypr_config/sync.rs`):
   açılışta + tema/shell.toml değişiminde üret/yaz/source/uygula; `msg hypr status|sync|add-source|undo|apply-switch`;
   source eklenince log + masaüstü bildirimi. Ortam kapısı (`SyncEnv::guard`): Hyprland imzası
   yoksa kapalı; test örneği yalnızca `~/.config` dışındaki `XDG_CONFIG_HOME`'a yazar, Hyprland'in
   komut satırına bakmaz ve Hyprland'e komut göndermez (testli; tek istisna iç içe test
   Hyprland'i: `NUXUT_TEST_HYPR_CONTROL=1` + `/run/user` dışında `XDG_RUNTIME_DIR`).
4. **Yapıldı:** install.sh nuxut.conf yazmaz ve hyprland.conf'a dokunmaz; `--no-hyprland`
   shell.toml'a `[compositor] ensure_source = false` yazar. Kaldırmada durum işareti silinir.
   `nuxut-shell hypr write` (çevrimdışı üretim) gerekmedi: ilk açılış üretir.
5. Ayarlar: §6 sayfaları (ayrı işçi).
