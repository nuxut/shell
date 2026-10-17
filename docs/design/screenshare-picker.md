# Ekran paylaşım seçicisi (`nuxut-sharepicker`)

(t126 ilk yarısı). İşçi, 2026-10-14.

Bir uygulama ekran paylaşımı istediğinde (xdg-desktop-portal `ScreenCast`, Hyprland'de arka uç
xdg-desktop-portal-hyprland = xdph) xdph bir **seçici süreci** başlatır ve ondan ne paylaşılacağını
öğrenir. Varsayılan seçici Qt'li `hyprland-share-picker`; xdph `screencopy:custom_picker_binary`
ile başka bir ikiliye izin verir. `nuxut-sharepicker` o sözleşmeye uyan, framework bileşenleriyle
çizilmiş kendi seçicimiz. Paylaşımın kendisi (PipeWire akışı, ekran kopyalama) xdph'te kalır.

## Sözleşme (xdph 1.4.1)

**Kaynak:** ağ yok ve `references/` altında xdph kaynağı yok; sözleşme bu makinede kurulu
paketlerden (`xdg-desktop-portal-hyprland 1.4.1-2`, Hyprland 0.56.2) **ikililerin
disassembly'siyle** doğrulandı: `/usr/lib/xdg-desktop-portal-hyprland` (sembolleri kısmen
duran `promptForScreencopySelection()`, `sanitizeNameForWindowList()`) ve
`/usr/bin/hyprland-share-picker` (çıktıyı yazan Qt slot'ları). "Doğrulandı" = ikilide görüldü;
"doğrulanmadı" = ikilide görülmedi ya da yalnızca dolaylı kanıt var.

### Çağrı (xdph → seçici)

| Ne | Değer | Durum |
|---|---|---|
| İkili | `screencopy:custom_picker_binary` boşsa `hyprland-share-picker`, değilse o yol | doğrulandı |
| Argüman | yalnızca `--allow-token`, `screencopy:allow_token_by_default` açıksa | doğrulandı |
| Ortam | `WAYLAND_DISPLAY`, `XCURSOR_SIZE` (yoksa 24), `HYPRLAND_INSTANCE_SIGNATURE`, `QT_QPA_PLATFORM=wayland`, `XDPH_WINDOW_SHARING_LIST` eklenir | doğrulandı (adlar ve `addEnv` çağrıları) |
| Diğer ortam (`HOME`, `XDG_*`) | Hyprutils `CProcess` üst sürecin ortamını devralıyor gibi | **doğrulanmadı** (framework temasını `XDG_CONFIG_HOME`/`HOME`'dan okuruz; xdph systemd kullanıcı servisi olduğu için bu değişkenler orada var) |
| Bekleme | `runSync()`: xdph seçici bitene kadar o isteği bekler | doğrulandı (çağrı); xdph'in başka istekleri de bekleyip beklemediği **doğrulanmadı** |

### Pencere listesi (`XDPH_WINDOW_SHARING_LIST`)

Girdiler art arda: `<id>[HC>]<sınıf>[HT>]<başlık>[HE>]<adres>[HA>]` (biçim dizesi
`{}[HC>]{}[HT>]{}[HE>]{}[HA>]`, argüman türleri ikilide `u32, string_view, string_view, u64`).

- `id`: toplevel tutamacının alt 32 biti (ondalık). Seçimde aynen geri verilir.
- `sınıf`, `başlık`: `sanitizeNameForWindowList` ile `'` `"` `$` `` ` `` → boşluk ve `>]`
  dizisindeki `]` → boşluk (ikilide doğrulandı). Bu yüzden işaretler metinde geçemez, bölme tek
  anlamlı. Sınıf/başlık sırası işaret adlarından (HC = class, HT = title) çıkarıldı; ikilide
  birinci metin argümanı tutamaç nesnesinin 0. ofsetindeki, ikincisi 0x20 ofsetindeki dizgedir
  (sınıf-önce bir struct ile tutarlı), ama struct tanımı görülmediği için sıra **tam
  doğrulanmadı** (canlı xdph ile bir kez bakılmalı; ters ise yalnızca gösterim etkilenir, seçim
  kimlikle yapılır).
- `adres`: Hyprland pencere adresi (u64 ondalık). Seçici yalnızca bilgi olarak tutar.
- Liste boş ya da değişken yoksa: xdph toplevel dışa aktarma yeteneği olmadan listeyi boş verir.

Bizim sınırlarımız (`services::sharepicker`): değişken > 128 KiB (Linux `MAX_ARG_STRLEN`) ise
tümden reddedilir; en çok 256 pencere; kimlik yalnızca ASCII rakam, u32; yinelenen kimlik, eksik
işaret ve sondaki yarım girdi atlanır (sayılır, loglanır; başlıklar loglanmaz). Gösterimde
denetim/yön/sıfır genişlikli karakterler atılır, sınıf 96, başlık 200 karaktere kırpılır.

### Çıktı (seçici → xdph)

stdout'ta tek satır: `[SELECTION]<bayraklar>/<seçim>\n`.

| Seçim | Biçim | Durum |
|---|---|---|
| Ekran | `screen:<çıkış adı>` (wl_output adı, ör. `DP-1`) | doğrulandı; xdph sondaki `\n`'i `pop_back()` ile atar — **satır sonu zorunlu** |
| Pencere | `window:<id>` (listedeki kimlik) | doğrulandı; xdph `stoull` ile okur |
| Bölge | `region:<çıkış>@<x>,<y>,<genişlik>,<yükseklik>` | doğrulandı (`@` ve `,` ayırıcıları, `stoi`) |
| Bayrak `r` | uygulamaya geri yükleme belirteci (restore token) verilsin | doğrulandı; bilinmeyen bayrak xdph'te yalnızca loglanır |

- Bölge koordinatları **çıkışa göre yerel, mantıksal piksel**: `hyprland-share-picker` slurp'ün
  düzen koordinatından `QScreen::geometry()` (mantıksal) konumunu çıkarıp yazıyor (doğrulandı).
  xdph'in bu değerleri ölçekle nasıl çarptığı **doğrulanmadı**; biz başvuru seçicisiyle aynı
  birimi yazıyoruz.
- `hyprland-share-picker` pencere kimliğini `int` olarak yazar (negatif çıkabilir; xdph `stoull`
  sarar). Biz `u32` ondalık yazarız (aynı alt 32 bit).
- Çıkış adında `@`, `/`, `,`, boşluk, denetim karakteri olan çıkış listelenmez (sözleşmeye
  yazılamaz).

### İptal

xdph stdout'ta `[SELECTION]` aramaz bulamazsa isteği iptal sayar (doğrulandı). Çıkış kodunu
kullanıp kullanmadığı **doğrulanmadı**; biz iptalde boş stdout + çıkış kodu 1, hatada 2, seçimde 0
veririz. Seçici asılı kalırsa xdph isteği de asılı kalacağından 180 sn etkileşimsizlikte iptal.

### Ayar dosyası

`$XDG_CONFIG_HOME/hypr/xdph.conf` (yoksa `$HOME/.config/hypr/xdph.conf`; ikilide iki yol ve
"neither $HOME nor $XDG_CONFIG_HOME" iletisi var), hyprlang:

```
screencopy {
    custom_picker_binary = /home/<kullanıcı>/.local/lib/nuxut/nuxut-sharepicker
}
```

xdph ayarı yalnızca açılışta okur: değişiklik bir sonraki girişte ya da
`systemctl --user restart xdg-desktop-portal-hyprland` ile etkin. hyprlang'de aynı kategorinin
ikinci kez açılması (kullanıcının kendi `screencopy { … }` bloğunun yanına bizimkini eklemek)
Hyprland'in kendi ayarında olağan; xdph'in hyprlang'inde **canlı doğrulanmadı**.

## Uygulama

- **İkili:** `nuxut-shell` paketinin ikinci ikilisi (`src/bin/nuxut-sharepicker.rs`), kurulumda
  `$PREFIX/lib/nuxut/`. Ayrı workspace üyesi değil: bölge seçim durum makinesi ekran
  görüntüsüyle ortak (`services::screenshot::selection`) ve kabuk kataloğu (`shell-sharepicker-*`)
  kullanılır. stdout yalnızca sözleşme satırı; loglar stderr.
- **Sözleşme katmanı:** `src/services/sharepicker.rs` (saf, UI'sız; ayrıştırıcı, çıktı biçimi,
  argümanlar, çıkış adı doğrulaması, bölge yuvarlama). Birim testleri bozuk/uzun girdiyi kapsar.
- **Görünüm:** `src/sharepicker_view.rs`. Odaklı çıkışta layer-shell `Overlay` (tam ekran,
  `KeyboardMode::Exclusive`, karartılmış zemin) + framework `Dialog`. Sekmeler `SegmentedButton`
  (Ekran / Pencere / Bölge), listeler `List`/`ListItem`, "Bu izni hatırla" anahtarlı `ListItem`.
  - Ekran: çıkış adı; alt metin çıkış açıklaması · boyut · yenileme (boyut **mantıksal**;
    fiziksel kip çözünürlüğü framework'te yok — talep aşağıda).
  - Pencere: başlık (boşsa "Adsız pencere") + sınıf; ön ikon sınıfın Nerd Font glifi
    (`ui::app_glyph`, yoksa genel pencere ikonu). Önizleme yok (aşağıda).
  - Bölge: "Paylaş" kartı kapatır, her çıkışa bir `Overlay` seçim katmanı açar (klavye kartın
    çıkışında). Sürükle: bölge (taşı/boyutlandır, ok tuşları); sürüklemesiz tıklama ya da seçimsiz
    Enter: o çıkışın tamamı (bölge olarak). Esc/sağ tık karta döner. Canlı ekran karartılır (kare
    dondurulmaz).
  - "Bu izni hatırla": `--allow-token` ile açık başlar; açıksa çıktıya `r` bayrağı.
- **Klavye:** Esc iptal, Enter paylaş, Sol/Sağ sekme, Yukarı/Aşağı liste (odak = seçim), Tab
  liste ↔ "Bu izni hatırla", Space anahtarı çevirir. Seçimsiz Enter/Paylaş kartı kapatmaz, durum
  satırında uyarı. **Yazı-önü kapısı:** kart ilk sunulduktan sonra 400 ms tuşlar yok sayılır
  (başka pencerede basılmakta olan Enter ekranı paylaşmasın; polkit diyaloğuyla aynı gerekçe).
  Ön seçim yok: yanlışlıkla paylaşım olmasın diye kullanıcı açıkça seçer (karar,
  2026-10-14; onayına açık).
- **Dil:** `shell-sharepicker-*` anahtarları (tr kaynak, en).
  Bölge boyut etiketi `shell-screenshot-size`'ı yeniden kullanır.

## Kurulum

`install.sh` ikiliyi `$LIBEXEC`'e kurar ve `dist/xdph/xdph-picker.sh install <yol>` çağırır:

- Blok işaretli: `# >>> nuxut-sharepicker: … >>>` … `# <<< nuxut-sharepicker <<<`; dosyanın geri
  kalanı korunur. İdempotent (değişiklik yoksa hiçbir şey yazılmaz), yol değişince yalnızca blok
  güncellenir.
- Yazmadan önce yedek: `xdph.conf.nuxut-yedek-<zaman>` (yalnızca içerik değişiyorsa).
- **Dokunulmaz:** blok dışında `custom_picker_binary` varsa (kullanıcının kendi seçicisi), dosya
  sembolik bağsa (dotfile yöneticisi; elle ekleme yönergesi yazdırılır), işaretler bozuksa, yol
  hyprlang için güvenli değilse (`[A-Za-z0-9/._+-]` dışı).
- `--no-sharepicker` adımı atlar; `--dry-run` `plan` ile ne olacağını yazar; `--uninstall`
  yalnızca bizim bloğu kaldırır (dosyada başka bir şey yoksa dosyayı siler; önce yedek).
- Testler geçici `XDG_CONFIG_HOME` ile (`tests/it/xdph_picker_script.rs`); adım kullanıcının
  gerçek dosyasına geliştirme sırasında çalıştırılmadı.

## Kapsam dışı (ilk yarı)

- Pencere önizlemesi (küçük resim): framework'te güvenli bir toplevel yakalama yolu yok
  (hyprland-toplevel-export ya da ext-image-copy-capture istemcisi) → framework talebi.
- Bölge katmanında pencere vurgusu/tıkla-pencere: pencere geometrisi için Hyprland IPC gerekir;
  ilk yarıda yok (tıklama tüm çıkışı seçer).
- Gerçek paylaşım oturumuyla uçtan uca deneme yapılmadı (görev kuralı); sözleşme ikili analizi +
  birim testleri + izole sway'de seçicinin stdout'u ile doğrulandı.

## Framework talepleri

1. `OutputInfo`'ya geçerli kipin fiziksel boyutu (`mode_size`) — ekran listesinde gerçek
   çözünürlük (şu an mantıksal boyut gösteriliyor).
2. Toplevel önizleme yakalama (hyprland-toplevel-export-v1 / ext-image-copy-capture-v1) — pencere
   listesinde küçük resim; ekran görüntüsü de kullanır (`docs/design/screenshot.md`).
3. Uygulama sınıfından (app_id) tema ikonu çözümü (freedesktop ikon teması) — Nerd glifi olmayan
   uygulamalarda gerçek ikon.

## Kararlar (2026-10-14; kullanıcı onayına açık)

- **Ön seçim yok, seçimsiz Enter işlemez** (400 ms kapı dahil): ONAY — paylaşım açık seçim ister.
- **`allow_token_by_default` `shell.toml`'a taşınmaz.** Varsayılan kapalı; izin yalnızca seçicideki
  "Bu izni hatırla" anahtarıyla verilir (xdph'in kendi ayarı açıksa anahtar açık başlar).
- **Canlı xdph doğrulaması** (sınıf/başlık alan sırası, bölge ölçek birimi, ikinci `screencopy { }`
  bloğunun birleşmesi) kullanıcının oturumunda gerekir → kullanıcıya sorulmak üzere PARK; o zamana
  dek bu üç nokta "doğrulanmadı" olarak kalır.
- Framework talepleri (yukarıda) framework'e iletildi (2026-10-14).
