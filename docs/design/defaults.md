# Sözleşme: varsayılan uygulamalar v1 (t116) — taslak 2026-10-13

Shell ve Ayarlar arasında. Ayarlar YALNIZCA shell.toml + framework.toml yazar (sözleşme 1), bu yüzden
`mimeapps.list` yazımı shell'de (`nuxut-apply`, snapshot'lı). Ayarlar `msg defaults status` okur,
`msg defaults set` gönderir. Aday listesi ve mevcut seçim tek kaynak: shell (launcher'ın XDG
desktop indeksi + mimeapps çözümü).

## Roller ve MIME kümeleri
| rol | birincil (mevcut seçim buradan) | yazılan küme (uygulamanın bildirdikleriyle KESİŞİM) |
|---|---|---|
| browser | x-scheme-handler/https | x-scheme-handler/http, x-scheme-handler/https, text/html, application/xhtml+xml |
| email | x-scheme-handler/mailto | x-scheme-handler/mailto |
| file_manager | inode/directory | inode/directory |
| terminal | — (MIME yok, aşağıya bak) | — |
| text_editor | text/plain | text/plain |
| image | image/png | image/png, image/jpeg, image/gif, image/webp, image/bmp, image/tiff, image/svg+xml, image/avif, image/jxl |
| video | video/mp4 | video/mp4, video/webm, video/x-matroska, video/quicktime, video/mpeg, video/x-msvideo, video/ogg |
| music | audio/mpeg | audio/mpeg, audio/flac, audio/ogg, audio/x-vorbis+ogg, audio/opus, audio/wav, audio/x-wav, audio/mp4, audio/aac |
| pdf | application/pdf | application/pdf |

- Aday: `Type=Application`, `Hidden` değil, `TryExec` (varsa) PATH'te, birincil MIME'ı `MimeType`'ta
  bildiren `.desktop` (XDG_DATA_HOME + XDG_DATA_DIRS, kimlik = XDG desktop-id, ilk bulunan kazanır).
  `NoDisplay=true` aday OLABİLİR (işleyiciler) ama listede sona. Ada göre sıralı (katalog dili).
- terminal: aday = `Categories` içinde `TerminalEmulator`. `xdg-terminal-exec` PATH'teyse yazım
  `~/.config/xdg-terminals.list` (tek satır desktop-id başa; diğer satırlar korunur); yoksa
  `available: false` (rol gösterilir, "xdg-terminal-exec kurulu değil" notu; seçim yazılmaz).

## Mevcut seçimin çözümü (xdg-mime query default eşdeğeri, Rust)
Sıra: `$XDG_CONFIG_HOME/<masaüstü>-mimeapps.list` (XDG_CURRENT_DESKTOP öğeleri, küçük harf) →
`$XDG_CONFIG_HOME/mimeapps.list` → `$XDG_CONFIG_DIRS` (aynı ikili) → `$XDG_DATA_HOME/applications/`
→ `$XDG_DATA_DIRS/applications/` (mimeapps.list + `<masaüstü>-`), `[Default Applications]` ilk
KURULU giriş; hiçbiri yoksa `mimeinfo.cache` ilk aday (source = "system_cache"). `[Removed Associations]`
uygulanır.

## Yazım
- YALNIZCA `$XDG_CONFIG_HOME/mimeapps.list` `[Default Applications]`: yazılan kümedeki ve uygulamanın
  bildirdiği her MIME için `mime=<id>;` (önceki değer tamamen değişir). Diğer bölümler, satırlar,
  yorumlar, sıra korunur (nuxut-apply IniKey parçaları). Dosya yoksa yaratılır.
- Snapshot: SettingId `defaults.<rol>` (kapalı kimlik listesine eklenir), `msg snapshot restore` ile geri.
- Masaüstüne özgü dosya (`<masaüstü>-mimeapps.list`) aynı MIME'ı tanımlıyorsa yazım YİNE yapılır ama
  status `shadowed_by` (dosya yolu, `~` kısaltmalı) döner; Ayarlar uyarır. O dosyaya dokunulmaz.

## IPC
`nuxut-shell msg defaults status` → JSON v1:
```json
{ "v": 1,
  "roles": [
    { "role": "browser",
      "current": { "id": "firefox.desktop", "name": "Firefox", "icon": "firefox" } ,
      "source": "user|desktop_user|system|system_cache|none",
      "mixed": false,
      "shadowed_by": null,
      "available": true,
      "candidates": [ { "id": "firefox.desktop", "name": "Firefox", "icon": "firefox", "hidden": false } ] } ] }
```
- `current` null = hiçbir şey açmıyor. `current.id` aday listesinde olmayabilir (kurulu değil →
  `name` = id). `mixed`: yazılan kümedeki MIME'lar (uygulamanın bildirdikleri arasında) birden çok
  uygulamaya gidiyor. `hidden` = NoDisplay. Roller tablo sırasıyla, hepsi her zaman var.
- `nuxut-shell msg defaults set <rol> <desktop-id>` → `{"ok":true,"snapshot":"<id>","written":[mime…]}`
  ya da hata kodu: `unknown_role`, `unknown_app` (aday değil), `unavailable` (terminal, araç yok),
  `writer_disabled`, `write_failed`. Değişiklik yoksa `ok` + boş `written`, snapshot yok.
- Zaman aşımı: Ayarlar 2 sn bekler (status ilk taramada yavaşsa shell önbellekli indeksi kullanır).
- Dosya izleme yok; Ayarlar pencere odağı dönünce yeniden sorar (input gibi).

## Ayarlar sayfası
Sistem grubunda "Varsayılan uygulamalar" (PageId::DefaultApps). Rol başına satır: ikon + ad açılır
seçici (adaylar, NoDisplay olanlar "Diğer" ayracından sonra). `mixed` → not "Bazı dosya türleri başka
uygulamada açılıyor" (seçim hepsini birleştirir); `shadowed_by` → uyarı notu; terminal `available:false`
→ not, seçici kapalı. Snapshot "↺ önceki" mevcut satır geri alma deseniyle.

## Uygulama kararları (2026-10-13; karar veren, onayına sunuldu)

Sözleşmenin belirsiz bıraktığı yerlerde en sade yorum seçildi:

1. **Kod yeri.** `src/services/default_apps/` (`mod.rs` rol tablosu, indeks, status, set;
   `mimeapps.rs` okuma/çözüm; `terminal.rs`), IPC `src/ipc.rs` `DefaultsCommand`, çalışma anı
   `src/app/defaults.rs` (`nuxut-defaults` thread'i, ilk komutta açılır, istekler sırayla).
2. **Ayrıştırıcı tek.** Launcher'ın `desktop.rs`'ine `MimeType` ve `Categories` eklendi
   (kopya ayrıştırıcı yok). `MimeType` için ayrı sınır: değer 32 KiB, 1024 öğe (VLC ~6 KiB,
   genel 4 KiB / 64 öğe sınırı medya oynatıcıların türlerini kesiyordu). `Scanner::scan_index`
   görünürlük süzgeci olmadan kimlik başına ilk girdiyi verir; launcher'ın `scan`'i bunun üzerine
   kurulu (davranış aynı). Worker kendi `Scanner`'ını tutar (launcher thread'iyle paylaşılmaz;
   (dev, ino, boyut, mtime) önbelleği sayesinde ilk taramadan sonra yalnızca `stat`).
3. **Kurulu** = indekste, `Type=Application`, `Hidden` değil (TryExec bakılmaz; `xdg-mime`
   gibi). **Aday** = kurulu + `TryExec` (varsa) bulunuyor + birincil MIME bildiriyor (terminal:
   `Categories` ∋ `TerminalEmulator`). `OnlyShowIn`/`NotShowIn` ve `Exec` yokluğu süzülmez
   (sözleşmede yok). Sıra: `NoDisplay` sonda, sonra ad (küçük harf), sonra kimlik.
4. **`[Removed Associations]`.** Bir dosyanın kaldırmaları ondan **sonraki** dosyaların
   `[Default Applications]`'ını ve `mimeinfo.cache` yedeğini etkiler; aynı dosyadaki açık
   varsayılan kaldırmadan güçlüdür (kullanıcı aynı dosyada ikisini de yazdıysa açık seçim kazanır).
5. **`source`.** `$XDG_CONFIG_HOME/mimeapps.list` → `user`; `$XDG_CONFIG_HOME/<masaüstü>-…` →
   `desktop_user`; eski kullanıcı konumu `$XDG_DATA_HOME/applications/(<masaüstü>-)mimeapps.list`
   → `user` (kullanıcının dosyası); `$XDG_CONFIG_DIRS`, `$XDG_DATA_DIRS` → `system`.
6. **`mixed`.** Geçerli uygulamanın bildirdiği, yazılan kümedeki MIME'ların çözümleri (hiçbir
   şey = ayrı bir hedef) birden fazla ise `true`. Geçerli uygulama yoksa `false`.
7. **`shadowed_by`.** Yalnızca `$XDG_CONFIG_HOME/<masaüstü>-mimeapps.list` (bizim yazdığımız
   dosyadan önce okunan tek kullanıcı dosyası) rolün yazılan kümesinden bir MIME için
   `[Default Applications]` girdisi taşıyorsa; yol `nuxut_apply::Env::display_path` ile (`~`).
8. **`set` yazımı.** Yalnızca **değişen** MIME'lar parça olur ve `written`'a girer (zaten
   `<id>;` olanlar yazılmaz); hiçbiri değişmiyorsa `{"ok":true,"snapshot":null,"written":[]}`,
   snapshot yok. Kaynak `Source::Settings` (istemci Ayarlar). Etiket katalogdan
   (`shell-defaults-snapshot-label`). Snapshot geri yüklemesinin canlı karşılığı yok.
9. **Hata biçimi.** `{"ok":false,"error":"<kod>"}`; `write_failed`'da ek `"detail"`
   (`nuxut-apply` sonuç/hata adı: `busy`, `changed`, `externally_managed`, `io`…; yol/değer yok).
   `writer_disabled`: ikincil örnek (`Env::disabled`) ya da `nuxut-apply` kapısı yazımı
   atladı (test örneğinde kök dışı hedef). Biçimce geçersiz kimlik (`[A-Za-z0-9._+-]`,
   `.desktop` sonu, ≤ 255) `unknown_app`. Komut sözdizimi hatası (eksik argüman) diğer msg
   komutları gibi `error: kullanım: …`.
10. **Terminal.** `current`: `xdg-terminals.list` zincirinde (her dizinde `<masaüstü>-` önce;
    config home → config dirs → `$XDG_DATA_HOME/xdg-terminal-exec` → `$XDG_DATA_DIRS/
    xdg-terminal-exec`) ilk **kurulu** girdi; satırdaki `:eylem` / `/…` eki kimlikten ayrılır.
    Liste yoksa `current: null`, `source: "none"` (xdg-terminal-exec'in kendi yedeği taklit
    edilmez). `shadowed_by`: kullanıcının `<masaüstü>-xdg-terminals.list`'i kurulu bir girdi
    taşıyorsa. `mixed` hep `false`. `written` = `["xdg-terminals.list"]` (MIME yok).
11. **Terminal yazımı (uygun parça türü yok → en sade güvenli yol).** `nuxut-apply` `Line`
    parçası var/yok'tur, satır taşımaz. Kimlik dosyada yoksa tek işlem: `Line{at: Start}`
    (dosya yoksa yaratılır). Kimlik ilk girdiyse değişiklik yok. Kimlik **daha aşağıdaysa** iki
    işlem: önce o satır kaldırılır (`Source::Shell`), sonra başa eklenir (`Source::Settings`;
    kaynak farkı iki işlemin birleşmesini önler — birleşseydi güvenli geri yükleme çakışma
    görürdü). Son snapshot'ın geri yüklenmesi eklemeyi geri alır → önceki ilk girdi yine
    geçerlidir; kimliğin eski (etkisiz) alt satırı geri gelmez (ilk snapshot da geri yüklenirse
    sona eklenir). Kalıcı çözüm framework talebi olabilir: "ilk satır olsun" kipli `Line`.
12. **Önbellek / 2 sn.** İlk `status` tam tarama yapar; sonrakiler yalnızca `stat`. Ön ısıtma
    yok (boşta iş yok ilkesi). Ölçüm (2026-10-13, bu VM, debug derleme, `/usr/share` +
    `/usr/local/share` gerçek uygulama dizinleri, `msg` gidiş-dönüş dahil): ilk `status` 131 ms,
    ikinci 23 ms — 2 sn sınırının çok altında.
13. **Kaynak.** WIP (e60128b, kapanış), `default_apps` tarafından gözden geçirilip
    tamamlandı; sözleşmenin repodaki adı `docs/design/defaults.md` (görev,
    2026-10-13). Sözleşme metninden sapma yok; yukarıdaki maddeler yalnızca sözleşmenin açık
    bıraktığı yerleri doldurur.
14. **inceleme düzeltmeleri (2026-10-13; uygulayan).**
    - *Panik yok:* snapshot ayar kimliği (`defaults.<rol>`) bulunamazsa `unreachable!` yerine
      `write_failed`/`unknown_setting` (worker thread'i ölürse sonraki her komut "thread kapalı"
      dönerdi). Test: her rolün kimliği kapalı listede, tablo sırasıyla.
    - *Terminal taşımada telafi:* kaldırma yapılıp başa ekleme düşerse (`busy`, `changed`, `io`…)
      kaldırma snapshot'ı hemen geri yüklenir; satır listenin **sonuna** döner (etkin terminal —
      ilk girdi — zaten değişmemişti). Yanıt yine `write_failed` + ayrıntı. Geri yükleme de düşerse
      uyarı izi (bilinen sınır; kalıcı çözüm "ilk satır" kipli `Line`).
    - *`[Added Associations]` (madde 4'e ek):* hiçbir dosyanın `[Default Applications]`'ı karar
      vermezse zincir sırasıyla `[Added Associations]`'daki ilk kurulu kimlik (kaynağı o dosyanın
      `source`'u), o da yoksa `mimeinfo.cache`. Kaldırmalar aynı kuralla (yalnızca sonraki
      dosyalar). Herhangi bir dosyanın varsayılanı her Added'dan güçlüdür (XDG mime-apps
      belirtimi; `gio` ile aynı yön).
    - *Yinelenen `[Default Applications]` bölümü:* okuma GLib gibi bölümleri birleştirir (sonraki
      değer kazanır); `nuxut-apply` `IniKey` ise yalnızca ilk bölümü düzenler. Yazım etkisiz
      kalabileceği için böyle bir dosyaya **yazılmaz**: `write_failed` + `"detail":
      "duplicate_section"`, dosyaya dokunulmaz, snapshot yok (bilinen sınır; kullanıcı dosyayı
      düzeltince çalışır).
    - *Aday kimliği:* adaylar `set`'in kabul ettiği biçimle (madde 9) süzülür — listede görünüp
      `unknown_app` dönen aday olmaz (ör. boşluk ya da ASCII dışı karakterli kimlik listede yok).
    - *İz:* okunamayan `mimeapps.list`/`mimeinfo.cache` (yokluk dışında) `debug` izi bırakır (yol yok).

## Ek: `msg defaults open <rol>` (ADR-0006, 2026-10-15)
Temel katmanın terminal / dosya yöneticisi / tarayıcı kısayolları için: rolün geçerli varsayılanı
(yukarıdaki çözüm; kurulu değilse yok sayılır), yoksa görünür ilk aday (ada göre). Yanıt
`{"ok":true,"id":"…","fallback":false}` ya da `{"ok":false,"error":"unknown_role|no_app"}`; başlatma ana
thread'de başlatıcı servisiyle (etkinleştirme belirteci yok; odak compositor'ın).
