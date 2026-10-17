# ADR-0007: Hyprland ana ayarının devralınması (benzersiz yedekli)

- Durum: kabul edildi (2026-10-16). Karar sahibi: **kullanıcı, 2026-10-16**:
  "hyprland.conf Nuxut'u göstermiyorsa yedekle, Nuxut'u gösteren yeni bir ayar oluştur.
  Yedekler sınırsız olabilir ve birbirini yok etmemeli." Tasarım ayrıntıları
  kullanıcıya anlatıldı (2026-10-16).
- Bağlam: kullanıcının makinesinde `hyprland.conf` end4'ün (dots-hyprland) dosyalarını yüklüyor;
  Nuxut'un 11 kısayolu (ses, parlaklık, medya, Print, SUPER+.) çakışma kuralıyla (ADR-0006) düşüyor.
  Tek satır eklemek (ADR-0003/0006) başkasının tam masaüstü ayarı varken yetmiyor.
- İlgili: ADR-0006 (katman sırası), `docs/design/hyprland-takeover.md`, kod
  `src/services/hypr_config/takeover.rs`.

## Kararlar

1. **Ölçüt:** ana dosyanın ilk boş olmayan satırı `# nuxut: managed main config v1`
   (`-- …` Lua). Bir `source`/`require` satırı yetmez (end4 dosyası Nuxut satırını da içerebilir).
   İşaret yoksa devralınır.
2. **Yedek:** `<ad>.nuxut-yedek-YYYY-MM-DD-HHMM`, aynı dakikada `-2`, `-3`… `O_EXCL` ile yaratılır (sıra no sınırsız)
   (hiçbir dosyanın üzerine yazılmaz), `fsync`, geri okunup baytlar karşılaştırılır; uyuşmazsa kendi
   yarım kopyamız silinir ve devralma yapılmaz. Sayı sınırı yok; Nuxut yedek silmez. Kullanıcının dosya
   izinleri korunur.
3. **Yeni ana dosya** (sıra ADR-0006 ile aynı mantık): işaret ve açıklama → temel katman satırı
   (ADR-0006'nın birebir satırı) → önceki zincirdeki `monitor =` satırları (`$değişken` açılmış;
   açılamayan atlanır) / Lua'da üst düzey `hl.monitor(…)` çağrıları → Ayarlar katmanı (ADR-0003
   satırları) → **en sonda kullanıcının dosyası** `user.conf` / `require("user")` (`user.lua`).
   Kullanıcı dosyası yoksa yalnızca açıklamayla oluşturulur; Nuxut ona bir daha yazmaz. Ad İngilizce
   (`user.*`): yapılandırma dosya adları Nuxut'ta İngilizce (`shell.toml`, `nuxut.conf`); içindeki
   açıklama Türkçe.
4. **Etkin dosya:** Hyprland'in seçimi — `hyprland.lua` varsa o, yoksa `hyprland.conf` (0.56.2
   `Jeremy.cpp getMainConfigPath`, ADR-0003 Bulgu 1; `SyncEnv::config_path` aynı sırayı uygular).
   Etkin olmayan dosyaya dokunulmaz.
5. **Doğrulama:** yeni içerik aynı dizinde aday dosyaya yazılır (göreli yollar aynı çözülür),
   `Hyprland --verify-config -c <aday>` (compositor/soket başlatmaz; başvuru §8; temiz ortam, 20 sn)
   "config ok" demezse eski dosya yerinde kalır, hata raporlanır. Yerine koyma `nuxut-apply`
   `write_file` (atomik rename, symlink hedefi korunur, okunan hâl değiştiyse vazgeçer). Nuxut'un
   dosyaları yoksa varsayılanlarla üretilir (hyprlang olmayan `source`'u hata sayar; boş katman
   oturumu kısayolsuz ve kabuksuz bırakırdı — tasarım belgesi adım 5).
6. **Geri alma:** `nuxut-shell hypr restore-backup` (CLI, çalışan kabuk gerekmez),
   `nuxut-shell msg hypr restore-backup` (çalışan kabukta; ardından yeniden yükleme),
   `install.sh --hypr-restore`. En yeni yedek (zaman damgası, sonra sıra no) geri konur; o anki dosya
   önce yine benzersiz yedeklenir. Tekrar çalıştırmak bir önceki hâle döner (yedekler hiç silinmez).
7. **Ne zaman:** yalnızca açık istekle: `install.sh --hyprland-takeover` ya da ilk kurulumda Hoş geldin'de
   kullanıcının onayladığı adım. Kurulum ve güncelleme bunu kendiliğinden ASLA yapmaz (2026-10-16:
   test dönemi taze kurulum kipi kalktı); `--no-hyprland` `--hyprland-takeover`'ı da kapatır. Kabuk açılışta kendiliğinden
   devralmaz.
8. end4'ün (ya da başka bir ayarın) diğer dosyalarına dokunulmaz; yalnızca artık yüklenmezler.

## Sonuçlar

- Devralınan makinede bütün Nuxut kısayolları çalışır; kullanıcının eski bind'leri `user.conf`'a
  elle taşınana kadar yoktur (yedekte durur).
- Taşınanlar (2026-10-17 ekiyle): monitörler, `env`/`envd`, `input`/`cursor`/`xwayland`/`render`/
  `opengl`/`device` ayarları. `workspace =`, `exec-once`, bind ve kurallar taşınmaz (eski
  kurulumun başlattığı programlar artık başlamaz — bilinçli: Nuxut masaüstünün kendisi); sayıları
  raporda ve bildirimde.
- Test örneği (`NUXUT_SHELL_INSTANCE`) yalnızca geçici `XDG_CONFIG_HOME` altında çalışır;
  gerçek `~/.config/hypr`'ye asla.

## Ek (2026-10-17): donanım/klavye/ortam taşınması, rapor, Hoş geldin onayı

Karar sahibi: **kullanıcı, 2026-10-17** — seçenek A (tam devralma, onaylı) kalır; gerçek makinedeki
riskler (NVIDIA + Intel çift GPU ortam değişkenleri, Türkçe klavye düzeni, VM pano betiği) kapatılır.

1. **Taşınanlar** eski zincirden (hyprlang; `source` yerinde açılır, `$değişken` açılmış, yorum
   atılmış) ayrı `user-migrated.conf`'a yazılır ve ana dosyada Nuxut ayarlar katmanından ÖNCE
   `source` edilir: Ayarlar'daki seçim (ör. klavye düzeni) bunların üstüne yazar; kullanıcının sonra
   kendi yazdıkları `user.conf`'ta (en son) kalır. Kapsam: bütün `env`/`envd` satırları;
   `input`, `cursor`, `xwayland`, `render`, `opengl`, `device` blokları (iç içe dahil) ve yollu
   atamaları (`input:kb_layout = tr`). Lua: üst düzey `hl.env(…)` çağrıları ana dosyaya.
2. Taşınan dosya da adayla doğrulanır (`--verify-config`); geçmezse hiçbir şey yazılmaz. Önceki
   devralmanın farklı `user-migrated.conf`'u önce benzersiz yedeklenir.
3. **Rapor** (JSON): `migrated {env, settings}`, `migrated_file`, `not_migrated {exec[], binds,
   rules, other}`, `clipboard_bridge: external_not_migrated` (eski ayarda `wl-paste` + `xclip`/`xsel`
   köprü betiği varsa; Nuxut köprüsü işini yapar). CLI raporu `$XDG_STATE_HOME/nuxut/hyprland-main-config.json`'a
   yazar ve çalışan kabuğa `msg hypr takeover-done` gönderir; kabuk özet bildirimi gösterir,
   "Geri al" = `msg hypr restore-backup`.
4. **Hoş geldin onayı:** Ayarlar'ın Hoş geldin "Masaüstüyle ortak" adımında, ana ayar henüz
   Nuxut'un değilse ve Hyprland oturumundaysa "Hyprland ayarımı Nuxut yönetsin" anahtarı seçili gelir;
   Bitir'e basılınca (shell `setup done`'u aldıysa) `nuxut-shell hypr takeover` çalışır. Hoş geldin
   yalnızca `setup.toml` yokken açılır (gerçek ilk kurulum). `msg setup status` `hypr.main`
   (`managed|available|missing|unknown`).

## Güvenlik (2026-10-17, inceleme bulgusu sensitive-data-exposure)

`env` satırları ve `exec` argümanları belirteç/anahtar içerebilir. Kural:
- Değerler yalnızca `user-migrated.conf`'ta (Hyprland okusun diye) durur. Rapor (JSON, durum
  dizinindeki dosya), bildirim, günlük, `msg hypr status` ve doctor çıktısında yalnızca ADLAR
  vardır: `migrated.env_names`, `not_migrated.exec` = program adı (`parse::program_name`;
  `AD=değer` önekleri, `sh -c`, yol ve bütün argümanlar atılır).
- Hyprland doğrulama hatası satırın değerini yansıtabilir: rapora yalnızca dosya adı ve satır
  numarası girer (`redact_verify_line`).
- Kullanıcı içeriği taşıyan yeni dosyalar yalnız sahibinin: yedekler kaynağın izniyle ama en çok
  `0600` (`private_mode`), `user-migrated.conf`, `user.conf` ve rapor `0600`; durum dizini `0700`.
- Test: `secrets_never_leave_the_migrated_file_and_files_are_private` (sahte `env = SECRET_TOKEN,abc123`
  ve `exec-once` argümanı; JSON, rapor dosyası, özet, doğrulama hatası; dosya izinleri).
