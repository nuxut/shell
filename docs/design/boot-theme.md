# Açılış görünümü: Plymouth, GRUB ve Limine temaları

İstek: kullanıcı, 2026-10-16 ("Plymouth, GRUB, Limine için tam teşekküllü temalarımızı yapalım;
başlangıç kurulumunda 'temaları ez' seçeneğiyle sağlanmalı"). Kararlar tarihli. Giriş ekranı ayrı
iştir; aynı kök yardımcıyı kullanır.
Kök yardımcı: [ADR-0008](../adr/0008-root-system-helper.md).

## Parçalar

| Parça | Yer | Yetki | Ne yapar |
|---|---|---|---|
| `nuxut-boot-theme` | `crates/nuxut-boot-theme` (UI'sız lib) | — | Tema dosyalarını bellekte üretir; saf ayrıştırıcı/düzenleyiciler (Limine bloğu, `plymouthd.conf`, mkinitcpio HOOKS/ön ayar, grub.cfg denetimleri); algılama ([`Probe`] üzerinden); plan; PF2 okuyucu ve önizleme çizici |
| `nuxut-system-helper` | `crates/nuxut-system-helper` (küçük ikili) | root (pkexec) | Yalnız beyaz listedeki yollara yazar, her şeyi yedekler, günlükler, doğrular, hata olursa geri alır |
| `nuxut-shell system-theme` | `src/services/system_theme.rs` | kullanıcı | Algılama + plan gösterimi, seçili paletten renkler, ölçek, önizleme PNG'leri, pkexec ile yardımcıyı çalıştırıp olayları akıtma |
| polkit eylemi | `dist/polkit/org.nuxut.system-helper.policy` | — | `org.nuxut.system-helper.boot-theme`, `auth_admin`, `exec.path` + `exec.argv1 = boot-theme` |
| `nuxut-shell system-theme install-helper` | `src/services/system_bundle.rs` | root (pkexec bsdtar, tek onay) | Kaynak kurulumunun hazırladığı kök dosyalarını (sabit liste) kurar; `apply` yardımcı yoksa ya da eskiyse önce bunu yapar (ADR-0008 ek, 2026-10-17) |
| `install.sh --system-theme` / `--system-theme-revert` | kaynak kurulumu | sudo yalnız eksik `greetd` paketi için | Planı gösterir, onay ister, dosyaları hazırlar, uygular (kurulum ve uygulama yönetici onayıyla) |

## Kararlar

1. **Plymouth: two-step modülü, kendi betik teması değil** (2026-10-16). LUKS parola
   diyaloğu, büyük harf kilidi, klavye düzeni göstergesi, çoklu ekran ve sıcak takma two-step'te
   (Fedora/GNOME `spinner`/`bgrt`) sınanmış C kodudur. Betik (`script`) modülündeki bir hata parola
   sorusunu görünmez bırakabilir: kullanıcı kör yazar ya da açılamaz. Düzen: ortada Kara Sağan
   (watermark, 128 px), altında 200×4 px ilerleme çubuğu (açılış) ya da 28 px sakin dönen gösterge
   (kapanış/yeniden başlatma, 36 kare); parola diyaloğu kilit ikonu + yuvarlatılmış alan + noktalar.
   Animasyon ölçülü: tek renk, parlama/iz yok, geçiş efekti yok (`Transition=none`).
2. **Renkler hep seçili paletin koyu kipinden** (2026-10-16). Önyükleme
   ekranı karanlık ürün yazılımı ekranının ardından ve çoğu zaman karanlık odada gelir; açık zemin
   gece göz kamaştırır. Türetme GTK/Qt'ye giden rol kümesiyle aynı (`look::palette::build`): koyu
   kipteyken Nuxut'un kendi token'ları birebir; duvar kağıdı paletinde tohum `palette.toml`'dan.
   Logonun gövde tonu framework `brand_shade(primary)`; logo geometrisi framework varlığından
   (`nuxut-ui/assets/brand/kara-sagan.svg`, kopya yok; varlık değişirse test kırılır).
3. **GRUB teması damla dosyasıyla** (`/etc/default/grub.d/90-nuxut-theme.cfg`, yalnız
   `GRUB_THEME=`). Kullanıcının `/etc/default/grub`'ı hiç değişmez; grub-mkconfig damlaları ondan
   sonra okur (son atama geçerli). Tema `/boot/grub/themes/nuxut/` altında: GRUB kök dosya
   sistemini (şifreli/btrfs alt birimi) okuyamayabilir, `/boot`'u her zaman okur. Zemin düz renk
   (`desktop-color`): çözünürlükten bağımsız, büyük görsel yok. Seçili giriş `primary_container`
   hap (9 parçalı), metin `on_primary_container`; konsol/düzenleyici kutusu `surface_container` +
   ince çerçeve. Klavye: GRUB'un kendi tuşları; tema yalnız görünüş.
4. **GRUB fontu GRUB'un kendi aracıyla** (`grub-mkfont`): PF2 biçimi GRUB'un kodlayıcısından çıkar
   (önyüklemeyi bozacak bir biçim hatası riski yok). Kaynak yalnız sistem font dizinindeki sabit
   dosyalar (`fontsel`: Inter → Noto Sans → Adwaita Sans → DejaVu Sans → Liberation Sans; eş
   aralıklı JetBrains Mono → … → Liberation Mono); kullanıcı font dizini ve fontconfig kullanılmaz,
   kök yardımcı kullanıcının yazabildiği dosyayı `grub-mkfont`'a vermez. Üretilen PF2'nin adı geri
   okunup doğrulanır (`Nuxut Regular 16`).
5. **Ölçek (HiDPI)**: GRUB ve Limine çözünürlüğe uyamaz; ürün yazılımı genelde ekranın doğal
   çözünürlüğünü verir. Kullanıcı tarafı en büyük ekranın fiziksel yüksekliğinden (≥1800 → 2,
   ≥2800 → 3) ve Hyprland ölçeğinden büyüğünü seçer (1–3); tüm GRUB ölçüleri ve fontlar onunla
   çarpılır. Plymouth kendi `DeviceScale`'ini kullanır (varlıklar 1×; yukarı akış temalarıyla aynı),
   metni o ölçekte çizer.
6. **Limine: yalnız renk bloğu, duvar kağıdı yok** (2026-10-16). Limine menüsü terminal
   ızgarasına çizilir; çözünürlüğü bilmeden konan logo menüyle çakışır; görsel ayrıca yapılandırmanın
   bölümünde olmalı ve Secure Boot + kayıtlı sağlama toplamında hash ister. Blok
   (`# >>> nuxut-system-theme` … `# <<<`) dosyanın başına konur: marka satırı "Nuxut" `primary`,
   yardım `on_surface_variant`, terminal zemin/ön plan ve 16 renklik palet paletten. Blok dışındaki
   aynı anahtarlar `#nuxut-disabled# ` önekiyle yoruma alınır (geri almada açılır).
7. **systemd-boot temalanamaz** (gerekçeli durum): menüsü UEFI metin konsoludur; `loader.conf`'ta
   renk, yazı tipi ya da görsel seçeneği yoktur (yalnız `timeout`, `console-mode`, `default`…).
   Nuxut systemd-boot'a dokunmaz; Plymouth teması yine uygulanır (açılışın görünen kısmı odur).
   Önerimiz menüyü gizlemek (`timeout 0`, gerektiğinde boşluk tuşu) — kullanıcının kararı, otomatik
   yapılmaz. Plan bunu `systemd_boot: true` ile bildirir.
8. **Plymouth etkin değilse etkinleştirilmez** (v1). Paket kurmak, mkinitcpio HOOKS'a `plymouth`
   eklemek ya da çekirdek satırına `splash` yazmak açılış zincirini değiştirir; tema bunlardan
   ayrı ve zararsızdır. Plan eksikleri not olarak gösterir (`plymouth-not-in-initramfs`,
   `plymouth-no-splash`); etkinleştirme sonraki aşama (açık iş).
9. **İlk kurulumda varsayılan seçili** (kullanıcı ilkesi "Nuxut yapabiliyorsa varsayılan o"):
   Hoş geldin adımı uygulanabilir hedef varsa açık gelir; yönetici parolası yine her seferinde
   sorulur (`auth_admin`, keep yok).

## Akış

```text
Hoş geldin adımı / install.sh --system-theme
  └─ nuxut-shell system-theme status --json      algılama + plan + renkler + ölçek (yetkisiz)
  └─ nuxut-shell system-theme preview --out D    PNG önizleme (yetkisiz, sisteme dokunmaz)
  └─ nuxut-shell system-theme apply --json       pkexec /usr/lib/nuxut/nuxut-system-helper
        boot-theme apply --targets … --scale N --lang tr --color rol=#rrggbb ×14
        └─ polkit: org.nuxut.system-helper.boot-theme (nuxut-polkit penceresi)
        └─ yardımcı: kilit → kendi algılaması → plan olayı → hedef başına işlem → olaylar
```

Yardımcı **kendi** algılamasını yapar ve yalnız onun planını uygular; kullanıcı tarafından gelen
tek şey renkler, ölçek, dil ve hedef listesidir (dosya yolu ya da içerik asla).

### Hedef başına işlem (sıra GRUB → Limine → Plymouth)

Her hedef ayrı bir işlemdir: ya tamamen uygulanır ya da o turdaki her değişiklik ters sırayla geri
alınır (`Tx`); bir hedefin hatası diğerlerini etkilemez. Olay `target … rolled-back` ise sistem o
hedef için işlem öncesi hâlindedir; `inconsistent` yalnız geri almanın kendisi de başarısızsa
(mesajda yedek yolları).

- **GRUB**: tema dizini geçici kardeşte (`.nuxut.nuxut-new`) kurulur, PF2'ler üretilip doğrulanır,
  dizin atomik yeniden adlandırmayla yerine konur (önceki bizim değilse kalıcı yedeğe alınır) →
  damla dosyası → `grub-mkconfig -o grub.cfg.nuxut-new` → `grub-script-check` →
  `check_new_cfg` (üretilmiş başlık, giriş sayısı **azalmamış**, tema yükleniyor) → eski grub.cfg
  yedeklenir → atomik yeniden adlandırma. Elle yazılmış grub.cfg (grub-mkconfig başlığı yok),
  damla desteklemeyen eski GRUB, bizden sonra `GRUB_THEME` atayan bir damla → hedef engellenir.
- **Limine**: aday yollar (`/boot`, `/efi`, `/boot/efi` ve kök altında Limine'nin arama sırası +
  `EFI/<x>/limine.conf`); birden çok aday → engellenir (hangisinin okunduğu bilinmez). ESP'deki
  her `.EFI` ikilisinde `++CONFIG_B2SUM_SIGNATURE++` ardından 128 sıfır dışı karakter = kayıtlı
  sağlama toplamı → engellenir (dosya değişince Limine panikler). Dosyanın tamamı ayrıştırılır
  (tanınmayan tek satır, ör. eski `limine.cfg` sözdizimi → engellenir), blok eklenir, sonuç yeniden
  ayrıştırılır, yedek + atomik yazma + geri okuma.
- **Plymouth**: `/usr/share/plymouth/themes/nuxut/` (aynı dizin değişimi; sistemdeki `spinner`
  temasının `keymap-render.png`'si varsa kopyalanır) → `/etc/plymouth/plymouthd.conf` `[Daemon]
  Theme=nuxut` (diğer satırlar korunur) → HOOKS'ta `plymouth`/`sd-plymouth` varsa initramfs:
  ön ayarların ürettiği imgeler (`*_image=`/`*_uki=`, düz mutlak yollar; çözülemezse elle yeniden
  kurma notu) önce yedeklenir (dizin başına toplam + 64 MiB yer yoksa hedef uygulanmaz) →
  `mkinitcpio -P` (çıktısı satır satır gösterilir, 15 dk sınır) → başarısızsa imgeler geri konur;
  başarıda işlem yedekleri silinir (`/boot` dolmasın).

### Geri alma

`nuxut-shell system-theme revert` → `boot-theme revert`. Bildirimdeki (`/var/lib/nuxut/system-theme/
manifest.json`) kayıtlar ters sırayla: bizim dizinler kaldırılır (önceden duran yabancı dizin
yerine konur), kurduğumuz dosyalar içerikleri hâlâ bizimkiyse silinir, değiştirdiklerimiz
yedekten geri konur. Dosya bizden sonra değişmişse yalnız bizim parçamız geri alınır (Limine bloğu
kaldırılır ve yoruma alınan satırlar açılır; `plymouthd.conf`'ta tema eski değerine döner).
**grub.cfg ve initramfs eski kopyadan geri konmaz, yeniden üretilir** (araya çekirdek
güncellemesi girmiş olabilir; eski grub.cfg yeni çekirdeği bilmez) — yine doğrulanarak.

### Yedekler

`<ad>.nuxut-<YYYYmmddHHMMSS>[-n].bak`, özgün dosyanın yanında (elle kurtarma kolay; çakışmayan ad,
var olanın üstüne asla yazılmaz). Kalıcı yedek yalnız Nuxut öncesi özgün dosyanındır; yeniden
uygulama yeni kalıcı yedek almaz. Geri alma başarılıysa yedekler silinir.

## Tehdit modeli (kök yardımcı)

**Korunan:** sistemin açılabilirliği ve kök yetkisinin kapsamı. **Saldırgan:** kullanıcı oturumunda
çalışan kötü amaçlı kod (aynı uid) — yardımcıyı polkit onayı olmadan çalıştıramaz, ama onaylı bir
çalıştırmanın girdisini ve dosya sistemi durumunu etkilemeye çalışabilir. **Kapsam dışı:** root,
polkitd ve paket yöneticisi güvenilir; kullanıcının kendisinin onayladığı bir uygulama zaten yetkili.

| Saldırı | Savunma | Test |
|---|---|---|
| Yardımcı ikilisini değiştirme | `/usr/lib/nuxut/` kökün; polkit eylemi bu yola bağlı (`exec.path`); kullanıcı tarafı ikili ve dizini kökün değilse ya da yazılabilirse pkexec'i hiç çağırmaz (`HelperState::Unsafe`) | `helper_state_requires_root_ownership` |
| Başka bir alt komutu bu eylemle çalıştırma | `exec.argv1 = boot-theme`; alan başına ayrı eylem | — (polkit) |
| Kötü girdi | Yalnız argüman: ≤ 64 adet, ≤ 128 bayt ASCII, denetim karakteri yok; renkler `#rrggbb` ve 14 rolün her biri tam bir kez; ölçek 1–3; dil `tr`/`en`; hedef adları sabit. Dosya yolu ve içerik kabul edilmez | `args::rejects_bad_input`, `colors::*` |
| Yapılandırma enjeksiyonu (font adı vb.) | Plymouth/GRUB'a yazılan aile adı sabit listeden; yine de `[A-Za-z0-9 -]` denetimi | `files::names_and_families` |
| Sembolik bağ / dizin değişimi | Her yol kökten `openat(O_DIRECTORY\|O_NOFOLLOW)` ile yürünür; her dizin kökün ve grup/diğerlerine yazılamaz olmalı; dosyalar `O_NOFOLLOW`, yalnız düzenli dosya; bağın üstüne yazılmaz | `safefs::symlinks_are_refused`, `world_writable_dir_is_refused`, e2e `symlinked_theme_dir_is_refused` |
| TOCTOU | İşlemler doğrulanmış dizin tanıtıcısına göre (`openat`/`renameat`/`unlinkat`); geçici dosya `O_CREAT\|O_EXCL\|O_NOFOLLOW`; yeniden adlandırma aynı dizinde | `safefs::*` |
| Beyaz liste dışına yazma | `policy::allowed`: sabit yollar, Limine aday kalıpları, `/boot` ya da `/efi` altındaki `.img`/`.efi` imgeleri, bunların yedek ve geçici kardeşleri; her yazma/silme/yeniden adlandırma denetlenir. Bildirim (manifest) okunurken de her yol denetlenir | `policy::whitelist`, `manifest::round_trip_and_validation` |
| Dış komut ortamı | Mutlak yol (`/usr/bin/…`), `env_clear`, sabit `PATH`, `LANG=C.UTF-8`, cwd `/`, stdin kapalı; çıktı denetim karakterleri ayıklanıp satır başına sınırlı; süre sınırı aşılırsa öldürülür | `cmd::*` |
| Kullanıcının dosyalarına dokunma | Font kaynakları ve değiştirilen her dosya kökün olmalı | `edit_file` / `apply_grub` sahip denetimi |
| Eşzamanlı çalıştırma | `/run/nuxut-system-helper.lock` (`flock`, beklemez) | e2e `concurrent_run_is_refused` |
| Önyükleyiciyi bozma | grub.cfg ve limine.conf yerine konmadan doğrulanır; initramfs yedekli; hata = geri alma | e2e senaryoları |
| Panik | `unwrap` yok, `panic = "abort"`; e2e her çalıştırmada çıkış kodu var (sinyal yok) denetler | e2e |

**Bilinen sınırlar:** yarıda kesilen (güç kesintisi) bir işlemde dosyalar yarım değildir (atomik)
ama o hedefin bazı adımları uygulanmış kalabilir; bildirim her adımdan hemen sonra yazıldığı için
`revert` onları da geri alır. İşlem geri alındığında boş kalan yeni dizinler (`/boot/grub/themes`,
`/etc/default/grub.d`) silinmez (zararsız). `mkinitcpio` kendi içinde yarıda kalırsa imgeler
yedekten geri konur.

## Önizleme

`nuxut-shell system-theme preview --out <dizin> [--size GxY]`: `plymouth.png`,
`plymouth-password.png`, `grub.png`, `limine.png`. Aynı üretici çıktıları (PNG'ler, düzen
sabitleri) ve **GRUB'un kendi fontu** (kullanıcı yetkisiyle, rastgele adlı ve yalnız yoksa yaratılan 0700 geçici dizinde — güvenlik
incelemesi "insecure-temp-file", 2026-10-17 — `grub-mkfont`, PF2
okuyucumuzla glif glif) ile çizilir. Sınır: bu bir yeniden çizimdir — two-step hizalama formülü,
GRUB 9 parçalı kutu ve taban çizgisi kuralları kaynaktan alındı ama piksel farkı olabilir;
Plymouth metni gerçekte FreeType ile yumuşatılır; Limine terminal fontu yerine eş aralıklı PF2
kullanılır, seçili giriş vurgusu yaklaşıktır. Plymouth'un kendi test kipleri (`plymouthd --debug`,
x11 çizici) "Görsel doğrulama"da kurulmadan, kullanıcı ad alanında kullanıldı.

## Görsel doğrulama (2026-10-17; kullanıcının sistemi ve kök yetkisi kullanılmadan)

Üretilen tema dosyaları `cargo run -p nuxut-boot-theme --example export_theme -- <dizin> [ölçek]
[sans.ttf] [mono.ttf]` ile bir dizine yazılır (PF2'ler `grub-mkfont` ile, yardımcıyla aynı
aralıklar), sonra gerçek önyükleyicilere çizdirilir:

- **GRUB 2.16**: `grub-mkstandalone -O x86_64-efi` (tema bellek diskinde) → QEMU + OVMF, ekransız;
  QEMU monitörü `screendump` ve `sendkey e`/`c`. Bulgular ve düzeltmeler:
  1. Seçili hap komşu girdinin üstüne taşıyordu: GRUB seçili kutuyu içeriğin **dışına** 9 parçanın
     boyu kadar büyütür. Artık GRUB öğe yüksekliği `item_h − 2·köşe`, aralık `item_spacing +
     2·köşe`, `item_padding = 0`; yatay pay köşe + iç boşluk (`art::nine_slice_padded`). Hap 40 px,
     haplar arası 6 px, metin hap kenarından 22 px içeride — önizleme çizicisiyle aynı geometri.
  2. Öğe kutusu fontun satır yüksekliğinden küçük olunca GRUB alt uzantıları kesiyordu (`g`, `p`):
     hap köşesi 12 → 8 (framework `radius_small`), kutu 24 px ≥ 16 px Noto satırı.
  3. `@TIMEOUT_NOTIFICATION_MIDDLE@` `__timeout__` etiketinde çevrilmeden göründü; `@KEYMAP_SHORT@`
     İngilizce kalıyordu (GRUB Türkçe kataloğu çoğu kurulumda yok). İkisi de tema dilinde kendi
     metnimiz (`grub::timeout_text`, `%d` saniye; `grub::keymap_text`).
  4. Düzenleyici/komut satırı çerçevesi `?` kutularıydı: PF2 aralığına kutu çizgileri
     (`0x2500–0x257F`) eklendi (`grub::PF2_RANGES`, yardımcı ve örnek aynı sabiti kullanır).
  - Bilinen sınır: düzenleyici ve komut satırının içi siyah — gfxterm boş hücreleri
    `background_color` ile boyar ve bu tema dosyasından ayarlanamaz; çerçeve bizim
    (`terminal_box`). Kabul.
- **Plymouth 26 (two-step)**: resmî Arch paketi indirilip imzası pacman anahtarlığıyla
  (`gpgv`) doğrulandı, **kurulmadan** `bwrap` kullanıcı ad alanında (`/usr` üstüne bindirme, sahte
  kök) `plymouthd --no-daemon --debug` x11 çizicisiyle, ekransız Xvfb'de çalıştırıldı (tty = `script`
  pty'si, `plymouth.ignore-udev`). `show-splash` + `system-update --progress` ve
  `ask-for-password`: logo, ilerleme çubuğu, kilit + parola alanı + istem tasarımdaki gibi.
- **Limine 12.9**: resmî paket (imza doğrulandı) `BOOTX64.EFI` + örnek `limine.conf` + Nuxut bloğu
  → QEMU + OVMF: blok ayrıştırılıyor, marka/yardım/terminal renkleri uygulanıyor. Seçili girdi
  Limine'de ön/arka plan takasıdır (ayrı renk seçeneği yok).

Betikler (test aracı, depoda değil): QEMU monitör soketi + `socat`; Xvfb `:87`; hepsi kendi
geçici dizininde, bitince süreçler kapatılır.

## Hoş geldin sözleşmesi (Ayarlar)

Adım: **"Sistem görünümünü Nuxut'a geçir"** (başlık önerisi; metinler Ayarlar'ın kataloğunda).

1. Adım açılırken `nuxut-shell system-theme status --json` (arka planda; ~50 ms). Alanlar
   (`version: 1`): `plan.targets[] {target: plymouth|grub|limine, state: ready|applied|absent|
   blocked, notes[] {kind, …}, changes[] {kind: create|modify|run, what}}`, `plan.systemd_boot`,
   `actionable[]`, `helper: installed|missing|unsafe`, `helper_path`, `helper_installable`
   (hazırlanmış kök dosyaları var: yardımcı yoksa uygulama önce onları kurar), `helper_update`
   (kurulu dosyalar eski: "Güncelle" düğmesi → `system-theme install-helper`), `scale`, `lang`,
   `colors[[rol, "#rrggbb"]]`.
   - `actionable` boşsa adım gösterilmez (ya da "Bu sistemde değiştirilecek bir şey yok").
   - `helper != installed` ve `helper_installable` değilse adım "Bu sürümde kullanılamıyor" notuyla
     devre dışı (kullanıcıya komut yazdırılmaz).
2. Varsayılan **seçili** anahtar: "Önyükleme ve açılış ekranını Nuxut'a geçir". Altında hedef
   başına satır (Plymouth "Açılış ekranı ve disk parolası", GRUB/Limine "Önyükleme menüsü") ve
   **"Neler değişecek"** açılır listesi: `changes[]` aynen (yol listesi; `create` "eklenir",
   `modify` "değişir, önce yedeklenir", `run` "çalıştırılır"). `notes[]` kind → metin:
   `plymouth-not-in-initramfs` "Açılış ekranı etkin değil; tema kurulur, etkinleştirmek ayrı bir
   adım", `plymouth-no-splash`, `plymouth-manual-rebuild {command}` (komut kopyalanabilir),
   `grub-text-terminal`, `grub-hand-written`, `grub-cfg-unreadable`, `grub-no-dropins`,
   `grub-overridden {path}`, `limine-ambiguous {paths}`, `limine-unreadable`,
   `limine-unsupported {reason}`, `limine-enrolled {binary}`. `blocked` hedef soluk + neden.
   `plan.systemd_boot` → bilgi: "systemd-boot menüsü temalanamaz; açılış ekranı yine Nuxut".
3. Önizleme (Ayarlar v1'de henüz gösterilmiyor, 2026-10-17; komut hazır): `nuxut-shell system-theme preview --out $XDG_CACHE_HOME/nuxut/system-theme-preview
   --size <pencere için uygun, ör. 960x600>` → JSON `{dir, files}`; kartlarda gösterilir
   (`plymouth.png`, `plymouth-password.png`, `grub.png` ya da `limine.png`).
4. Uygulandı (settings 182751e, 2026-10-17): adımda uygulanacak iş varken birincil düğme "Uygula"; basınca `nuxut-shell system-theme apply --json` arka planda; stdout
   satır başına bir olay (`crates/nuxut-system-helper/src/events.rs`):
   `plan`, `step {target, step, state: start|done}`, `log {source, line}` (ayrıntılar açılır
   alanında), `warn`, `target {target, outcome: applied|reverted|rolled-back|skipped|inconsistent,
   message}`, son satır `done {ok, reboot_hint}`. Çıkış kodu: 0 tamam, 1 en az bir hedef
   başarısız, 2 yardımcı yok/kullanım, 3 yönetici onayı verilmedi (hiçbir şey değişmedi).
   polkit penceresi nuxut-polkit'tir; adım beklerken "Yönetici onayı bekleniyor" gösterir.
   mkinitcpio 10–60 sn sürebilir: ilerleme `step` olaylarıyla.
5. Sonuç: başarıda "Yeniden başlattığınızda görünecek" + **"Geri al"** düğmesi
   (`nuxut-shell system-theme revert --json`, aynı olaylar). `rolled-back` → "Uygulanamadı,
   hiçbir şey değişmedi" + `message`. `inconsistent` → hata + ayrıntı (yedek yolları mesajda).
6. Ayarlar → Görünüm altında da aynı satır (sonradan açma/geri alma); palet değişince otomatik
   yeniden uygulanmaz (her seferinde yönetici parolası gerekir) — "Renkleri tazele" düğmesi
   `state == applied` iken aynı `apply`.

## Giriş ekranı (greeter) ile ortak yardımcı

`nuxut-system-helper` alan tabanlıdır: `nuxut-system-helper <alan> <eylem> …`, alan başına polkit
eylemi `org.nuxut.system-helper.<alan>` (`exec.argv1 = <alan>`). Greeter için önerilen alan
`greeter` (greetd yapılandırması, `nuxut-greeter` kullanıcısı/dizinleri); aynı ilkeler: yalnız
argüman girdisi, kendi beyaz liste girdileri (`policy.rs`), aynı `safefs`/`ops`/`Tx`/bildirim
katmanı (bildirim dosyası alan başına ayrı), aynı olay biçimi. Alanı giriş ekranı işi yazar, bu
crate'e ek modül olarak.

## Nasıl denenir (VM'de, kullanıcıyla)

Önce: VM anlık görüntüsü (snapshot) alın. Sonra:

```sh
sh install.sh --system-theme --dry-run      # plan ve yönetici onayıyla çalışacaklar; hiçbir şey değişmez
nuxut-shell system-theme preview --out /tmp/nuxut-boot-onizleme   # görsellere bakın
sh install.sh --system-theme                # onay → (gerekirse greetd paketi) → pkexec (kurulum + uygulama)
sudo reboot
```

Bakılacaklar: GRUB menüsü (logo, seçili hap, geri sayım, `e` ile düzenleyici kutusu, `c` konsol);
Plymouth kuruluysa açılış ekranı ve (LUKS varsa) parola sorusu, büyük harf uyarısı; kapanışta
dönen gösterge. Limine kuruluysa renkler ve "Nuxut" başlığı.

Geri al:

```sh
sh install.sh --system-theme-revert         # ya da: nuxut-shell system-theme revert
```

GRUB menüsü hiç gelmezse (beklenmez: grub.cfg doğrulanmadan yazılmaz): GRUB komut satırında
`set theme=` ile tema kapatılabilir; ya da kurtarma ortamından `/boot/grub/grub.cfg.nuxut-*.bak`
yedeğini `grub.cfg` adıyla geri koyun ve `/etc/default/grub.d/90-nuxut-theme.cfg`'yi silin.

## Açık işler

- Plymouth'u etkinleştirme (paket, HOOKS, `splash`) — ayrı onaylı adım (karar 8).
- Gerçek donanımda doğrulama (HiDPI `DeviceScale`, LUKS akışı, çoklu ekran); GRUB, Limine ve
  Plymouth yazılımsal olarak doğrulandı ("Görsel doğrulama").
- dracut/booster initramfs: bugün yalnız "elle yeniden kur" notu.
