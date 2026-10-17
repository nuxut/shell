# Tasarım: Hyprland ana ayarının devralınması

Durum: uygulandı (2026-10-16; taşıma, rapor ve Hoş geldin onayı 2026-10-17). Karar: ADR-0007 (+ ek). Kod:
`src/services/hypr_config/takeover.rs`, tarama `parse.rs` (`Migration`), Ayarlar `pages/hypr_takeover.rs`.

## Akış (`nuxut-shell hypr takeover`)

1. Kapı: `nuxut-apply` dosya kapısı (test örneği yalnız geçici köklerde); Hyprland oturumu gerekmez.
2. Etkin ana dosya: `SyncEnv::config_path` (çalışan Hyprland'in `-c`/`HYPRLAND_CONFIG`'i, yoksa
   `hyprland.lua` → `hyprland.conf`).
3. Kullanıcı dosyası (`user.conf`/`user.lua`) yoksa açıklamayla oluşturulur (`O_EXCL`).
4. İlk satır işaretse: `already_managed` (yedek yok).
5. Nuxut'un `nuxut.conf|lua` ve `nuxut-base.conf|nuxut/base.lua`'sı yoksa varsayılanlarla üretilir
   (temel katman: varsayılan kısayollar + kurallar; ayarlar katmanı: yalnız kabuğu başlatan kanca).
   Gerekçe (2026-10-16): Hyprland yeni ana dosyayı hemen yeniden yükler; kabuk hiç
   çalışmamışsa boş katmanla oturum kısayolsuz kalır, kabuk da sonraki girişte başlamazdı. Kabuk
   açılınca ikisini kendi ayarıyla yeniden yazar.
6. Taşınanlar: hyprlang zincir taraması (temel dosyaya girilmez): `ChainReport::monitors` ana dosyaya;
   `ChainReport::migration` (bütün `env`/`envd`; `input`/`cursor`/`xwayland`/`render`/`opengl`/`device`
   blokları ve yollu atamaları; `$değişken` açılmış, yorum atılmış, `#` → `##`) `user-migrated.conf`'a;
   taşınmayanların özeti (exec komutları, bind/kural/öteki sayıları). Harici VM pano köprüsü betiği
   `clipboard::takeover::is_clipboard_bridge` ile tanınır. Lua: üst düzey `hl.monitor(…)` ve
   `hl.env(…)` çağrıları ana dosyaya.
7. Yedek: benzersiz, doğrulanmış (ADR-0007 §2).
8. Yeni metin (ve taşınan dosya, aday adla) → aday `.nuxut-aday-<pid>-<ad>` (özgün uzantıyla biter: Hyprland sözdizimini uzantıdan
   seçer, `.lua` olmayan Lua aday hyprlang sanılıp reddediliyordu) → `Hyprland --verify-config`
   (`HOME`/`XDG_CONFIG_HOME` devralmanın ortamından) → aday silinir →
   geçtiyse önce `user-migrated.conf` (öncekisi farklıysa benzersiz yedekle), sonra ana dosya
   `write_file` (okunan hâl beklenir).
9. CLI raporu `$XDG_STATE_HOME/nuxut/hyprland-main-config.json`'a yazar, çalışan kabuğa
   `msg hypr takeover-done` gönderir: kabuk özet bildirimi ("Geri al" = `msg hypr restore-backup`).

Sonuç JSON: `{"result":"taken_over|already_managed|inactive|error","config","backup","user_file",
"migrated_file","migrated":{"env","settings"},"not_migrated":{"exec","binds","rules","other"},
"clipboard_bridge","monitors","errors"}`; `restore-backup`: `restored|no_backup` + `restored` (geri konan yedek).

## Yeni `hyprland.conf` örneği

```
# nuxut: managed main config v1
# Nuxut — ana Hyprland ayarı; nuxut-shell oluşturdu (ADR-0007). Bu dosyayı elle düzenlemeyin:
# önceki dosyanız: ~/.config/hypr/hyprland.conf.nuxut-yedek-2026-10-16-1430 (geri almak: nuxut-shell hypr restore-backup).
# Kendi ayarlarınız: ~/.config/hypr/user.conf — en son okunur, buradaki her şeyi ezer; Nuxut ona yazmaz.

source = ~/.config/hypr/nuxut-base.conf  # Nuxut temel katmanı: önce okunur, sonraki satırlarınız ezer

# Önceki ayardan taşınan monitörler
monitor = DP-2, 2560x1440@144, 1920x0, 1

# Önceki ayardan taşınan donanım, klavye ve ortam ayarları (Ayarlar bunların üstüne yazar)
source = ~/.config/hypr/user-migrated.conf

# Nuxut: kabuk ayarları (ekleyen nuxut-shell; kapatmak için shell.toml [compositor] ensure_source = false)
source = ~/.config/hypr/nuxut.conf

source = ~/.config/hypr/user.conf
```

Eşitleme (ADR-0003/0006) bu dosyada iki satırı da `present` görür, satır eklemez; `msg hypr undo`
yine yalnızca Nuxut satırlarını kaldırır.

## Doğrulama

Birim testleri (geçici HOME): end4 benzeri zincir (source edilen `hyprland/default/keybinds.conf`,
`$değişken`li monitör), aynı dakikadaki eski yedekle çakışma (`-2`, eski yedek bozulmaz), bayt
karşılaştırma, doğrulama başarısızlığında eski dosya, Lua (öncelik, `hl.monitor` taşıma, yorum
içindeki çağrı atlanır, `.conf`'a dokunulmaz), geri alma (en yeni seçimi, o anki dosyanın yedeği,
hiçbir dosya silinmez), test örneği kapısı. Taşıma (2026-10-17): end4 benzeri donanım zinciri
(NVIDIA env, `envd`, iç içe `input`/`touchpad`, `cursor` blok + yollu atama, `xwayland`, `render`,
`device`, `$kb` değişkeni, VM pano köprüsü betiği), Hyprland kuruluysa gerçek `--verify-config`;
doğrulama başarısızken taşınan dosya yazılmaz; önceki taşınan dosyanın yedeği; Lua `hl.env`.
Gerçek `~/.config/hypr`'de çalıştırılmadı.

## Açık

- Hoş geldin adımı: uygulandı (2026-10-17; Ayarlar "Masaüstüyle ortak" adımı, seçili gelir).
- Eski ayardaki kişisel kısayolların `user.conf`'a önerilmesi (otomatik taşıma yok).
