# Tasarım: Hyprland temel katmanı

Durum: uygulandı (2026-10-15). Karar: ADR-0006. Başvuru: `docs/research/hyprland-0.56-reference.md`
(Hyprland 0.56.2; aşağıdaki "[ref §n]" oraya işaret eder). Kod: `src/services/hypr_config/{base,keybinds,generate,lua,sync}.rs`.

> Kullanıcı (2026-10-14): "Kutudan çıktığı gibi eksiksiz bir masaüstü, GNOME/KDE düzeyinde. Super
> tuşuna basmak başlatıcıyı açmalı. End4 iyi bir referans, birebir aynısı değil; ondan çok daha
> kaliteli, profesyonel, endüstri standartlarına uygun. Şov için patlamalar, çocuksu şeyler değil."
> "Kullanıcının hyprland.conf'u en son okunur ve istediğini ezer."

## 1. Katmanlar ve okuma sırası

```
hyprland.conf | hyprland.lua (kullanıcının)
  1  source = ~/.config/hypr/nuxut-base.conf   # Nuxut temel katmanı …   ← BAŞA, tek satır
     pcall(require, "nuxut.base")  -- …                                   (Lua: nuxut/base.lua)
  …  kullanıcının kendi satırları — temel katmanı ezer
  n  # Nuxut: kabuk ayarları …
     source = ~/.config/hypr/nuxut.conf                                  ← SONA (ADR-0003, 1A)
```

- **Temel katman** (`nuxut-base.conf` / `nuxut/base.lua`): bütün kısayollar (kabuk eylemleri +
  pencere/çalışma alanı yönetimi), pencere kuralları, Nuxut katman kuralları, yerleşim/jest/davranış
  varsayılanları. Kullanıcının dosyasından **önce** okunur: kullanıcı her değeri ve kuralı ezer,
  `unbind = MOD, tuş` (Lua: `hl.unbind`) ile bir Nuxut kısayolunu kaldırabilir.
- **Ayarlar katmanı** (`nuxut.conf` / `nuxut.lua`, değişmedi): Ayarlar'da kullanıcının kendi seçtiği
  değerler (köşe, aralık, kenarlık, gölge, bulanıklık, hareket, `[input]`'ta yazılanlar, ortam,
  başlatma kancası, ekranlar). Sonda okunur (ADR-0003 karar 1A): Ayarlar'daki son açık tercih
  uygulanır, `hyprland.conf` ezdiğinde `msg hypr status` `overrides` raporlar. Grup grup kapatılabilir
  (`[compositor] sync_*`). **Kullanıcı onayı bekliyor:** kullanıcının "hyprland.conf en son okunur"
  sözünü temel katmanla sınırlı yorumladık; Ayarlar'ın açık seçimleri sonda kalmaya devam ediyor (§9-1).
- Kısayol çakışması: kullanıcının zincirinde aynı tuş bağlıysa temel katman eylemini Nuxut **hiç
  üretmez** (çift tetikleme yok; §3.3). Kabuk eylemleri (ses, parlaklık, medya, ekran görüntüsü,
  emoji…) ise ayar katmanından önce okunan bind'i **devralır** (§3.4).

Neden iki dosya: tek dahil etme satırı ya başta ya sonda olabilir; "kullanıcı ezer" (varsayılanlar)
ile "Ayarlar'daki seçim uygulanır" (1A) aynı noktadan sağlanamaz.

### 1.1 Satırın yaşam döngüsü (sync.rs)

- Yalnızca Nuxut bağlıyken (sondaki satır var ya da bu turda ekleniyor) ve temel içerik istenirken
  (`[compositor] base_layer` ya da `sync_keybinds`). Satır **tek parça**: açıklama aynı satırın
  sonunda (iki ayrı `Position::Start` parçası snapshot geri yüklemesinde ters sıraya dönüyordu).
- Durum (`msg hypr status` → `base`): `present` (satırın kodu ana dosyada, ardında yalnız yorum
  olabilir), `added`, `disabled_by_user` (satır yorumlanmış: `# source = …nuxut-base.conf`,
  `-- pcall(require, "nuxut.base")`), `removed_by_user` (`$XDG_STATE_HOME/nuxut/hypr-base-added`
  işareti var ama satır yok), `off`. Kullanıcı kapattıysa/sildiyse kendiliğinden geri eklenmez;
  `msg hypr add-source` zorla ekler; `ensure_source` kapatıp açmak işaretleri siler.
- Yazım `nuxut-apply` ile, `hypr.source` snapshot'ı (ilk bağlanışta sondaki iki satırla aynı işlem;
  eski kurulumda ayrı işlem "temel katman eklendi"). Dosya satırdan önce yazılır (hyprlang olmayan
  `source`'u hata sayar). Yabancı (başlıksız) dosya varsa yazılmaz, satır eklenmez.
- "Geri al" (`msg hypr undo`) iki satırı ve iki dosyayı birlikte kaldırır.
- Canlı uygulama: temel dosya değiştiyse (kısayol, kural) `reload`; yalnızca Ayarlar katmanının
  değerleri değiştiyse eskisi gibi `keyword` batch / `eval`.
- hyprlang taraması temel dosyaya girmez (`ScanInput::skip`), Lua taraması `nuxut.*` modüllerine
  girmez: kendi bind'imiz kullanıcınınki sanılmaz.

## 2. Ne var, ne yok

Var: kısayollar (§3), pencere kuralları (§6), katman kuralları (§7), hareket (§4), yerleşim ve
davranış (§5), 3 parmak yatay jest. Yok: `cursor:no_hardware_cursors` (VM/sürücü işi, kullanıcının),
`debug:vfr` (varsayılan doğru; `misc:vfr` 0.56'da yok [ref §7]), monitör (Ekranlar'ın), klavye düzeni
(`[input]`), çıkış/oturum kapatma kısayolu (Oturum menüsünde onaylı).

## 3. Kısayollar

### 3.1 İlkeler

1. **Düzenden bağımsız çalışırlık** [ref §2.3]: Hyprland keysym bind'ini ilk düzenin
   değiştiricisiz sembolüyle eşler. Bu yüzden: rakamlar **tuş koduyla** (`code:10…19`; Türkçe Q ve
   AZERTY'de aynı fiziksel satır), harfler keysym ile (tuşun üstündeki harf — GNOME/KDE gibi
   mnemonik), oklar/Page Up-Down/Tab/Return/Delete/F4/Space düzenden bağımsız keysym. Noktalama
   keysym'i (`equal`, `minus`, `slash`…) kullanılmaz — Türkçe Q'daki SUPER+equal sorunu bundandır
   (testli: `defaults_avoid_layout_dependent_punctuation`). Tek istisna emoji `SUPER+.`
   (2026-10-13): ABD ve Türkçe Q'da çalışır, AZERTY'de `.` Shift ister.
2. **Tanıdıklık**: Windows/GNOME/KDE/macOS'ta ortak olan alışkanlık korunur (Alt+F4, Super+L,
   Ctrl+Alt+T, Super+E, Ctrl+Alt+Delete, Alt+Tab); döşemeli düzen gereği (yön, alan) tiling
   masaüstlerinin (Hyprland örneği, i3/sway) yerleşik alışkanlığı.
3. **Ezilebilirlik**: her eylem `shell.toml [keybinds]` anahtarıdır (Ayarlar → Kısayollar);
   `""` kapatır. Kullanıcının kendi bind'i her zaman kazanır.
4. **Güvenlik**: kilit dışı eylemler kilit ekranında çalışmaz (yalnızca ses/parlaklık/medya/büyüteç
   `l`); oturumu sonlandıran tek tuş yok.

### 3.2 Tablo

GNOME/KDE/Windows/macOS sütunları genel bilgiden yazıldı, bu VM'de doğrulanmadı (yalnızca karşılaştırma
amaçlı). "Hyprland örneği" sütunu: Hyprland 0.56.2 örnek ayarı aynı tuşu bağlıyor mu (o ayar duruyorsa
kullanıcınınki çalışır, Nuxut'unki üretilmez; testli: `defaults_are_distinct_and_stock_overlap_is_known`).

| Eylem (`[keybinds]`) | Varsayılan | GNOME | KDE | Windows | macOS | Hyprland örneği |
|---|---|---|---|---|---|---|
| `launcher` | **Super'e dokun** (Super_L/Super_R, bırakınca) | Super (Etkinlikler) | Meta (Başlat) | Win | Cmd+Space | — |
| `settings` | Super+I | Super+I yok (Ayarlar menüden) | — | Win+I | Cmd+, (uygulama) | — |
| `terminal` | Ctrl+Alt+T | Ctrl+Alt+T (Ubuntu) | Ctrl+Alt+T | — | — | Super+Q (kitty) |
| `file_manager` | Super+E | Super+E (Ubuntu: ev) | Meta+E | Win+E | — | Super+E ✓çakışır |
| `browser` | Super+B | — | — | — | — | — |
| `lock` | Super+L | Super+L | Meta+L | Win+L | Ctrl+Cmd+Q | — |
| `session_menu` | Ctrl+Alt+Delete | Ctrl+Alt+Del (oturum kapat) | Ctrl+Alt+Del | Ctrl+Alt+Del | — | — |
| `close_window` | Super+Q | Alt+F4 | Alt+F4 | Alt+F4 | Cmd+W/Q | Super+C ✗ (Q = terminal ✓çakışır) |
| `close_window_alt` | Alt+F4 | Alt+F4 | Alt+F4 | Alt+F4 | — | — |
| `force_kill` | Ctrl+Alt+Esc → pencereye tıkla (`hyprctl kill`; Esc vazgeçer) | — (xkill) | Ctrl+Alt+Esc (aynı tıklama kipi) | (Görev Yöneticisi) | Cmd+Opt+Esc | — |
| `fullscreen` | Super+F | (F11 uygulama) | — | (F11) | Ctrl+Cmd+F | — |
| `maximize` | Super+M | Super+↑ | Meta+PgUp | Win+↑ | — | Super+M = çıkış ✓çakışır |
| `toggle_floating` | Super+Shift+Space | — | — | — | — | Super+V |
| `pin` | Super+P | — | — | — | — | Super+P = pseudo ✓çakışır |
| `cycle_windows` / `_back` | Alt+Tab / Alt+Shift+Tab | Alt+Tab | Alt+Tab | Alt+Tab | Cmd+Tab | — |
| `toggle_split` | Super+J | — | — | — | — | Super+J ✓çakışır |
| `focus_*` | Super+←→↑↓ | (Super+←→ yarıya döşer) | (Meta+← döşer) | (Win+← döşer) | — | Super+oklar ✓çakışır |
| `move_*` | Super+Shift+←→↑↓ | Super+Shift+←→ (monitör) | — | Win+Shift+← (monitör) | — | — |
| `shrink_/grow_width/height` | Super+Ctrl+←→↑↓ (tekrar) | Alt+F8 (kip) | — | — | — | — |
| `workspace` | Super + 1…0 (tuş kodu) | Super+1…9 (Ubuntu: dock) | Ctrl+F1… | — | Ctrl+1… | Super+1…0 keysym ✓çakışır |
| `move_to_workspace` | Super+Shift + 1…0 | Super+Shift+Home… | — | — | — | Super+Shift+1…0 ✓çakışır |
| `workspace_next` / `_prev` | Super+PgDn / Super+PgUp | Super+PgDn/PgUp | Ctrl+Alt+→/← | Win+Ctrl+→/← | Ctrl+→/← | — |
| `move_to_workspace_next/_prev` | Super+Shift+PgDn/PgUp | Super+Shift+PgDn/PgUp | — | — | — | — |
| `workspace_last` | Super+Tab | (Super+Tab uygulama değiştirici) | — | (Win+Tab görev görünümü) | — | — |
| `scratchpad` / `move_to_scratchpad` | Super+S / Super+Shift+S | — | — | (Win+Shift+S ekran görüntüsü) | — | Super+S, Super+Shift+S ✓çakışır |
| `window_drag` / `window_resize_drag` | Super+sol / sağ fare sürükle | Super+sürükle | Meta+sürükle | — | — | aynı ✓çakışır |
| `workspace_scroll_next/_prev` | Super+tekerlek | — | — | — | — | aynı ✓çakışır |
| `media_pause` | XF86AudioPause → oynat/duraklat (kilitte de) | duraklat | oynat/duraklat | oynat/duraklat | — | XF86AudioPause ✓çakışır |
| (mevcut) ses, parlaklık, medya, ekran görüntüsü, pano, büyüteç, emoji | değişmedi | | | | | |

Gerekçeler (kararın özü; tam karşılaştırma tabloda):
- **Super'e dokunma = başlatıcı**: kullanıcı kararı; GNOME/KDE/Windows'un ortak davranışı.
  Hyprland'in bırakma bind'i araya girilen bind/tuş/fare girdisinde iptal olur [ref §2.4]; tek
  bilinen boşluk (bağsız tuş + Super önce bırakılır) nadirdir, belgelenmiştir.
- **Oklar odak/taşıma**: döşemeli düzende birincil iş pencereler arası gezinmedir; GNOME/Windows'un
  Super+oklar = "yarıya döşe" davranışı zaten Hyprland'in döşemesinin kendisidir. Taşıma Shift,
  boyut Ctrl ile aynı tuş ailesinde (öğrenilebilirlik). Super+Alt+↑/↓ büyütece ayrılmış.
- **Çalışma alanı komşuluğu PgUp/PgDn**: GNOME standardı; oklar pencereye ayrıldığı için çakışmaz.
  Dizüstünde Fn+↑/↓ ya da 3 parmak/ Super+tekerlek.
- **Kapatmanın iki kısayolu**: Super+Q tiling/macOS alışkanlığı ve dizüstünde Fn gerektirmez; Alt+F4
  evrensel. Hyprland örnek ayarı duruyorsa Super+Q onundur (terminal), Alt+F4 yine çalışır.
- **Super+Tab = son alan**: Hyprland dünyasının alışkanlığı; görsel uygulama değiştiricisi yok
  (Alt+Tab pencere döngüsü). **Kullanıcı onayı bekliyor** (GNOME'da uygulama değiştirici).
- **Terminal Ctrl+Alt+T**: Ubuntu/KDE standardı. **Kullanıcı onayı bekliyor** (tiling
  alışkanlığı Super+Return; istenirse yalnızca varsayılan değişir).
- **Uygulama açma xdg varsayılanıyla**: `msg defaults open <rol>` (Ayarlar → Varsayılan uygulamalar
  ile aynı çözüm: `mimeapps.list`, terminalde `xdg-terminals.list`); varsayılan yoksa adaylardan ilk
  görünür olan; başlatma kabuğun başlatıcı servisiyle (kapsam, ortam). Uygulama adı gömülmez.
- **Kilit `<kabuk> msg lock || loginctl lock-session`** (kilit düzeltmesi c696288 ile uyumlu, 2026-10-16): kabuk logind
  Session.Lock → Lock sinyali → `nuxut-lock`; logind yoksa kabuk `nuxut-lock`'u doğrudan başlatır.
  Kabuk çalışmıyorsa `loginctl lock-session` (kilit, boşta kalma servisiyle aynı yol). Bkz.
  `docs/design/lock-trigger.md`.
- **Oturum menüsü**: `msg session menu` oturum kartının bulunduğu ilk açık kenar panelini açar (yoksa
  sağ panel). Ayrı tam ekran oturum menüsü yok — **kullanıcı onayı bekliyor** (yeni yüzey).

### 3.3 Biçim ve çakışma (keybinds.rs)

- `Shape::Key` (`MOD+…+Tuş`), `Shape::Tap` (tek değiştirici: `launcher = "SUPER"` → `Super_L` ve
  `Super_R` bırakma bind'i), `Shape::Digits` (`workspace = "SUPER"` → on bind `code:10…19`).
- Çakışma "aynı fiziksel tuş" üzerinden: `code:10…19` ↔ keysym `1…0`, `prior` ↔ `page_up`,
  `next` ↔ `page_down`; dokunma bind'inde maske yok sayılır. Bir eylemin somut bind'lerinden biri
  çakışırsa eylemin **tamamı** düşer (aile yarım kalmaz). Rapor `msg hypr status` `conflicts`.
- hyprlang'da her bind `d` (açıklama) bayrağıyla `nuxut:<eylem>` taşır; Lua'da `description`.
- Kullanıcının Lua `code:` bind'i `hyprctl binds`'te görünmez [ref §2.2] → çakışma algılanamaz
  (bilinen sınır).

### 3.4 Kabuk kısayollarının devralınması (2026-10-16)

Kullanıcı ilkesi "Nuxut yapabiliyorsa varsayılan Nuxut" (2026-10-16). Gerçek makinede end4'ün
`hyprland/default/keybinds.conf`'u ses ±/sessiz, mikrofon, parlaklık ±, oynat/sonraki/önceki,
Print ve SUPER+. tuşlarını bağladığı için 11 Nuxut kısayolu çakışma kuralıyla düşüyor, OSD/medya
çalışmıyordu.

- **Kapsam:** yalnızca kabuk eylemleri (`!Action::is_base()`: Nuxut'un kendi özellikleri). Temel
  katman eylemlerinde (pencere/alan yönetimi, uygulama açma) kullanıcınınki kazanır (ADR-0006).
- **Koşul:** çakışan her kullanıcı bind'i ayar katmanından (`nuxut.conf`) ÖNCE okunuyor
  (`ChainReport::bind_order`; satır sona ekleniyorsa hepsi önce) ve değiştiricisi `unbind` ile
  yazılabiliyor. Sonra okunan bir bind (ör. devralınmış ana ayarın `user.conf`'u) Nuxut'unkini
  yeniden bağlardı → eylem devralınmaz, çakışma raporlanır. `nuxut.conf` yüklenmiyorsa (satır
  yok/kapalı) devralma yok.
- **Yazım:** `nuxut.conf`'un sonunda `unbind = <kullanıcının tuşu>` + Nuxut'un bind'i. Temel
  katmana konmaz: oradaki bind kullanıcınınkinden önce okunduğu için `unbind` onu da silerdi.
  Kullanıcının dosyasına hiçbir koşulda yazılmaz (yedek gerekmez: dosyası değişmeden durur).
- **Geri alma:** kısayol başına `[keybind_owner] <eylem> = "user"` (Ayarlar › Kısayollar "Benimkini koru", §3.5) ya da hepsi için `[compositor] take_over_keys = false` → sonraki turda
  `unbind` satırları kalkar, kullanıcınınki yeniden çalışır; `msg hypr undo` her şeyi kaldırır.
- **Rapor:** `msg hypr status` `taken_over: [{action, combo, file, line, reason, can_take}]` (kullanıcının değişmeyen
  bind'inin yeri), devralınamayanlar `conflicts`.
- **Lua:** kullanıcı bind'lerinin yeri bilinmiyor (`hyprctl binds`): devralma yok, eski kural.

### 3.5 Kısayol başına seçim ve çakışma raporu (2026-10-17)

Gerçek makinede (end4 düzeni: `hyprland.conf` → `hyprland/…keybinds.conf` + `custom/` + en sonda
`nuxut.conf`) `nuxut-shell doctor` "37 Hyprland kısayol çakışması ⚠" gösterdi (önceden 11). Tanı
(`sync.rs` `end4_like_chain_conflicts_are_intended`, `tests/fixtures/hyprland/end4-like`): yeni
kısayol eklenmedi, devralma çalışıyor — 12 kabuk eylemi (ses ±/sessiz, mikrofon, parlaklık ±,
oynat/duraklat/sonraki/önceki, Print, SUPER+.) devralınıyor; kalan 37'nin hepsi temel katman
eylemi (başlatıcı, pencere kapat/odak/taşı/boyutlandır, çalışma alanları, karalama alanı, uygulama
açma…), ADR-0006 gereği kullanıcınınki bilerek kazanıyor. Önceki 11'i (§3.4, kabuk eylemleri)
devralma çözdü; doctor ise temel katmanın bilinçli çakışmalarını da uyarı sayıyordu.

- **Seçim:** `shell.toml` `[keybind_owner] <eylem> = "nuxut" | "user"`. Öncelik: açık seçim >
  `take_over_keys` (yalnızca kabuk eylemleri) > varsayılan (kabuk → Nuxut, temel katman →
  kullanıcı). "nuxut" temel katman eylemini de devralır (`unbind` + Nuxut'unki, ayar katmanında;
  §3.4'teki koşullar aynen). Ayarlar › Kısayollar her çakışan satırda tek düğme gösterir:
  kullanıcınınki çalışıyorsa "Nuxut'unkini kullan", Nuxut'unki çalışıyorsa ya da devralınamıyorsa
  "Benimkini koru". Komut yazdırılmaz.
- **Rapor:** `msg hypr status` `conflicts[]` ve `taken_over[]` öğelerine `reason` ve `can_take`
  eklendi. `conflicts.reason`: `user_wins` (temel katman varsayılanı), `user_choice` (açık seçim ya
  da `take_over_keys = false`) → bilinçli; `read_after` (kullanıcının bind'i `nuxut.conf`'tan
  sonra okunuyor), `not_loaded` (`nuxut.conf` yüklenmiyor), `special_mods` (CAPS/MOD2…),
  `unlocated` (Lua: yer bilinmiyor) → Nuxut'unki isteniyor ama devralınamıyor. `can_take`:
  "Nuxut'unkini kullan" işe yarar mı (çakışan her bind'in engeli yok). `taken_over.reason`:
  `default` | `chosen`. Engellenmiş eylemde raporlanan yer engelli bind'inkidir.
- **doctor:** bilinçli çakışmalar ⚠ değil ℹ (`keys_user`: "N kısayolda Hyprland ayarınızdaki tuş
  geçerli (bilerek)"); devralınanlar ℹ (`keys_taken`); yalnız devralınamayanlar ⚠ (`conflicts`),
  her öğede dosya:satır ve neden; öneri Ayarlar'daki düğmeler. Nedeni olmayan (eski kabuk) öğe ⚠.
- **`read_after` için "Nuxut'unkini kullan" yok:** Hyprland'de sonra okunan bind önce yazılan
  `unbind`'ı boşa çıkarır; tek çözüm kullanıcının dosyasında satır taşımak olurdu (Nuxut'un ayar
  katmanının yerini değiştirmek kullanıcının sonraki ayarlarını da ezerdi). Satır "Benimkini koru"
  (bilgiye döner) ya da Nuxut'a başka tuş atanmasını önerir.

## 4. Hareket

Kaynak framework hareket token'ları; masaüstü için ayrı ama türetilmiş profil (`generate.rs`
`anim_plan`, iki arka uç aynı). Önceki eşleme (yay eğrisi, taşmalı bezier, 380 ms+) kaldırıldı.

- Eğriler (framework Material 3, taşmaz): `nuxut_decel` = vurgulu yavaşlama (giren), `nuxut_accel`
  = vurgulu hızlanma (çıkan), `nuxut_std` = standart (yer değiştiren).
- Süre: rolün oturma süresi hız çarpanından arındırılır, banda kıstırılır, hız yeniden uygulanır.
  Giriş/taşıma `spatial_fast` → [180, 250] ms; çıkış, soluklaşma, kenarlık, katmanlar `effects` →
  [120, 200] ms; çalışma alanı `spatial_fast` → [200, 300] ms. Sakin ön ayar: pencere 250 ms,
  çıkış 200 ms, alan 270 ms. Çıkış girişten kısadır (Material/GNOME ilkesi).
- Stiller: pencere `popin 92%` (hafif büyüme + soluklaşma; Hyprland'in stilsiz varsayılanı 5 px'ten
  büyür — "patlama" [ref §6]); çalışma alanı `slide` (yatay, yön duyarlı); karalama alanı
  `slidefadevert 15%`; katmanlar `fade`.
- Azaltılmış hareket (`motion.enabled = false` / `instant`) → `animations:enabled = false`.

## 5. Yerleşim ve davranış (`base.rs` `OPTIONS`)

| Anahtar | Değer | Gerekçe |
|---|---|---|
| `general:resize_on_border` | true | başlık çubuğu yok; kenardan boyut her masaüstünde var |
| `general:snap:enabled` | true | yüzen pencere kenar/komşu yapışması (GNOME/macOS) |
| `input:follow_mouse` | 2 | tıklayınca odak (GNOME/KDE/Windows/macOS); tekerlek imlecin altına. **Kullanıcı onayı bekliyor** (Hyprland varsayılanı odak-imleci-izler) |
| `dwindle:preserve_split`, `force_split = 2` | true, sağ/alt | öngörülebilir döşeme |
| `misc:disable_hyprland_logo`, `disable_splash_rendering`, `force_default_wallpaper = 0` | | zemin Nuxut'un; maskot duvar kağıdı ve logo yok |
| `misc:focus_on_activate` | true | geçerli etkinleştirme (bağlantı → tarayıcı) öne gelir |
| `misc:mouse_move_enables_dpms`, `key_press_enables_dpms` | true | kapalı ekranı girdi uyandırır |
| `misc:allow_session_lock_restore` | true | kilit istemcisi çökerse yeni kilit (`msg lock`, logind Lock) oturumu yeniden kilitleyebilir; kapalıyken Hyprland yalnızca "kilit çöktü" ekranı gösterir |
| `binds:hide_special_on_workspace_change` | true | alan değişince karalama alanı kapanır |
| `ecosystem:no_update_news` | true | Hyprland sürüm haberi masaüstünde açılmaz; bağış hatırlatması dokunulmadı (**kullanıcı onayı bekliyor**) |
| `gesture = 3, horizontal, workspace` | | dokunmatik yüzeyde alan değiştirme |

## 6. Pencere kuralları

| Kural | Etki | Not |
|---|---|---|
| `match:modal true` | float, center | kipli diyaloglar |
| portal dosya seçicileri (`xdg-desktop-portal-gtk|-gnome`, KDE portalı) | float, center | üst pencere ilişkisi yok |
| `nuxut-filechooser` | float, center, 960×640 | dosya seçicinin isteği |
| başlık `Open/Save/Select/Choose File(s)/Folder…`, `Save As` | float, center | İngilizce başlıklar |
| tarayıcı diyalogları: `File Upload` (Firefox), `… wants to save/open` (Chromium) | float, center | portalsız GTK/yerleşik diyaloglar |
| Firefox türevleri (`firefox`, `librewolf`, `zen`, `floorp`, `waterfox`) + başlık `Library` | float, center | sınıfla sınırlı: başlık tek başına çok genel |
| ekran paylaşım göstergesi: `… is sharing your screen/a window/a tab.` (Chromium), `Sharing Indicator` (Firefox) | float, pin, no_initial_focus, alt orta (16 px) | döşenip yer kaplamaz, odak çalmaz, her alanda görünür. Paylaşımın kendisi barın gizlilik hapında da görünür (`[bar.widget.privacy] screen`) |
| polkit ajanları (gnome, hyprpolkitagent, KDE, LXQt, MATE) | float, center | Nuxut'unki katman |
| başlık `Picture-in-Picture` | float, pin, keep_aspect_ratio, ekran genişliğinin ¼'ü 16:9, sağ alt köşe 32 px | |
| `nuxut-welcome` | float, center | boyut istemciden (720×640, min=max) |
| `nuxut-settings` | float, center, 1040×720 | Ayarlar'ın kendi varsayılan boyutu |
| `match:content ^(video|game)$` | idle_inhibit fullscreen | tam ekran video/oyun kilitlenmez; terminal vb. tam ekran kilitlenir (güvenlik) |
| XWayland sürükle-bırak kaynak penceresi | no_focus | Hyprland örneğindeki düzeltme |

Üst pencereli ve sabit boyutlu pencereleri Hyprland zaten yüzdürür [ref §4].

## 7. Katman kuralları

| Ad alanı | Animasyon | Bulanıklık |
|---|---|---|
| `nuxut` (tek tam ekran kabuk yüzeyi) | yok (içerik kendi hareketiyle) | evet |
| `nuxut-polkit` | fade | evet |
| `nuxut-wallpaper`, `nuxut-dim` | yok (kendileri canlandırır) | — |
| `nuxut-screenshot`, `nuxut-sharepicker`, `nuxut-exclusion` | yok | — |

Bulanıklık pencere bulanıklığını izler (`[windows] blur`); `ignore_alpha` = gölgenin en koyu
opaklığı + 0,05 (gölgenin arkası bulanıklaşmaz, panel zemini bulanıklaşır).

## 8. Doğrulama

- Birim testleri: üretilen satırlar (iki arka uç), tuş kodu ve dokunma açılımı, çakışma eşleri,
  varsayılanların birbirinden farkı, örnek ayarla bilinen çakışmalar, noktalama yasağı, hareket
  bantları ve hız, taşmasız eğriler, katman/satır yaşam döngüsü, snapshot geri yükleme, Lua tarama.
- `Hyprland --verify-config` (compositor başlatmaz [ref §8]): hyprlang ve Lua çıktıları "config ok";
  elle: `NUXUT_HYPR_DUMP=<dizin> cargo test --lib dump_for_verify -- --ignored` sonra
  `env -i HOME=$HOME PATH=/usr/bin XDG_RUNTIME_DIR=<geçici> Hyprland --verify-config -c <dizin>/hyprland.conf`.
- Canlı dokunma testi kullanıcının oturumunda yapılmadı (sentetik girdi yasak); iç içe Hyprland bu
  VM'de çalışmıyor (ADR-0003).

## 9. Kullanıcı onayı bekleyenler

1. Ayarlar katmanının sonda kalması (§1) — "hyprland.conf en son" yalnızca temel katman için.
2. `input:follow_mouse = 2` (tıklayınca odak).
3. Terminal Ctrl+Alt+T (alternatif Super+Return); Super+Tab = son alan; Super+M = ekranı kapla.
4. Ayrı oturum menüsü yüzeyi (şimdilik oturum kartının paneli).
5. Hyprland bağış hatırlatmasının kapatılması (dokunulmadı).
6. Hyprland örnek ayarı kullanan yeni kullanıcı: örnek ayarın bind'leri kazanır (§3.2'deki ✓'ler);
   ilk kurulumda örnek ayarı Nuxut'la uyumlu bir şablona çevirmek ayrı karar (kurulum).

## 10. dots-hyprland (end4) karşılaştırması

Kullanıcı ilkesi (2026-10-16): end4'e özel kod yok; temel katman onun işe yarayan özelliklerini
**genel olarak** kapsar. Kaynak: `../references/dots-hyprland/dots/.config/hypr/hyprland/{keybinds,
general,rules}.lua`. Eşitlik ekleri: 2026-10-16.
"✓" kapsanıyor, "≈" başka yolla/başka tuşla, "—" yok (gerekçeli).

| end4 | Nuxut | Durum |
|---|---|---|
| Super'e dokun: arama/başlatıcı | `launcher` Super'e dokun | ✓ |
| Ctrl+Alt+Del oturum menüsü | `session_menu` (oturum kartı paneli) | ✓ |
| Super+V pano, Super+. emoji | Super+Shift+V pano (Super+V yüzer pencere değil; Shift+V Windows alışkanlığı), Super+. emoji | ≈ / ✓ |
| Super+Shift+S bölge görüntüsü, Print tam ekran | Print bölge, Shift+Print ekran, Alt+Print pencere; Super+Shift+S karalama alanına taşı | ≈ (Print GNOME/KDE/Windows ortak tuşu) |
| Super+Shift+Alt+Q `hyprctl kill` (pencereye tıkla) | `force_kill` Ctrl+Alt+Esc, aynı tıklama kipi | ✓ (KDE tuşu; üç değiştiricili end4 tuşu ezberlenmez) |
| XF86AudioPlay/Pause/Next/Prev, kilitte de | `media_play`, `media_pause`, `media_next`, `media_prev` (`l`) | ✓ (`media_pause` eklendi) |
| ses/mikrofon tuşları, kilitte de | `volume_*`, `mic_mute` (kabuk OSD'li) | ✓ |
| Super+eksi/eşittir yakınlaştırma | `magnifier_in/out/reset` Super+Alt+↑/↓/Esc | ≈ (noktalama düzene bağlı: §3.1) |
| Super+Tab genel bakış | Super+Tab son alan | — genel bakış yüzeyi yok (ayrı özellik) |
| Super+/ kısayol listesi | Ayarlar → Kısayollar | ≈ (ayrı hile sayfası yok) |
| Duvar kâğıdı seçici, açık/koyu tuşu | Ayarlar → Görünüm | ≈ (tuş yok) |
| OCR, Google Lens, ekran çevirisi, metin özeti, renk seçici, ekran kaydı | — | — bulut hizmeti tuşları ürün dışı; renk seçici ve kayıt ayrı özellik |
| Uyku / kapat tuşları | oturum menüsü (onaylı) | — tek tuşla oturum sonlandırma yok (§3.1-4) |
| Uygulama tuşları (terminal, dosya, tarayıcı, editör, ofis, mikser, görev yöneticisi) | terminal, dosya, tarayıcı, Ayarlar (xdg varsayılanı) | ≈ (gömülü uygulama adı yok; editör/ofis/mikser kullanıcının) |
| `misc:allow_session_lock_restore = true` | aynı | ✓ (eklendi) |
| `session_lock_xray` | — | — kilitliyken masaüstü içeriği görünmez (güvenlik) |
| `on_focus_under_fullscreen = 2`, `focus_on_activate` | `focus_on_activate` | ≈ (tam ekranda yeni pencere davranışı Hyprland varsayılanı) |
| `swallow` kapalı, `vrr = 0`, `force_zero_scaling` | dokunulmuyor | — Hyprland varsayılanı/Ekranlar'ın işi |
| `input:follow_mouse = 1`, `natural_scroll`, tekrar hızı, numlock | tıklayınca odak (2); dokunmatik/klavye `[input]` | ≈ (§5) |
| 3 parmak jest alan değiştirme | aynı | ✓ |
| Aç/Kaydet diyalog başlıkları (`Open File`, `Select a File`, `Open Folder`, `Save As`) | `nuxut-open-save` | ✓ |
| `File Upload`, `… wants to save/open`, Firefox `Library` | `nuxut-browser-dialogs`, `nuxut-firefox-library` | ✓ (eklendi; Library yalnız Firefox türevleri) |
| KDE portal diyalogu float + boyut | `nuxut-file-chooser` float + center | ✓ |
| Resim içinde resim | `nuxut-pip` | ✓ |
| `… is sharing your screen/a window` float + pin + alt orta | `nuxut-share-indicator` (+ Firefox Sharing Indicator, odak çalmaz) | ✓ (eklendi) |
| pavucontrol, nm-connection-editor, blueberry, kcm_* float | — | — uygulama adı gömülmez; Nuxut'un kendi panelleri var. Kipli/sabit boyutlu olanları Hyprland zaten yüzdürür |
| Bütün pencerelerde bulanıklık kapalı, `suppress_event maximize` | — | — Nuxut'un görünüm seçimi (`[windows] blur`); büyütme isteği uygulamanın hakkı |
| `.exe` başlıklı pencerelerde tearing | — | — oyun ayarı kullanıcının (`user.conf`) |
| Quickshell'e özel katman kuralları | Nuxut katman kuralları (§7) | ✓ (kendi ad alanlarımız) |
| exec: geoclue, gnome-keyring, hypridle, easyeffects, cliphist, imleç | kabuk servisleri (boşta, pano, saat dilimi, görünüm/imleç) | ≈ (Nuxut kendisi; gnome-keyring/easyeffects kullanıcının) |
