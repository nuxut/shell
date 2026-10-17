# Giriş ekranı (`nuxut-greeter`)

Karar: `docs/adr/0010-login-screen.md` (greetd + kendi greeter'ımız, Hyprland içinde;
2026-10-16). Kod: `greeter/` (çalışma alanı üyesi), kurulum dosyaları `dist/greeter/`.

## 1. Çalıştırma

```text
systemd ─▶ greetd (root, VT1)
            └─ default_session, kullanıcı `greeter`:
               /usr/lib/nuxut/nuxut-greeter-session        (dist/greeter/nuxut-greeter-session)
                 XDG_CONFIG_HOME=/var/lib/nuxut-greeter/config   (yayımlanmış tema, §5)
                 $XDG_RUNTIME_DIR/nuxut-greeter/{hyprland.lua,input.lua}
                 exec Hyprland --config …/hyprland.lua
                   └─ hyprland.start: nuxut-greeter; hyprctl dispatch 'hl.dsp.exit()'
```

- `nuxut-greeter` `GREETD_SOCK`'a bağlanır; oturum başlatılınca (`start_session` → `success`)
  içerik solar (≤ 400 ms) ve 0 ile çıkar; Hyprland ardından kapanır; greetd seçilen oturumu
  kullanıcı olarak başlatır.
- Her çıkışta tam ekran katman yüzeyi (`overlay`, özel klavye). Arka plan kilit ekranının
  görünümüdür; form yalnızca "ev sahibi" çıkışta (klavye odağı, odak gelmeden işaretçinin olduğu
  çıkış) — kilitle aynı kural.
- Greeter ayarı (`hyprland.lua`) Lua'dır (ADR-0003); `Hyprland --verify-config` ile denetlendi.
  Animasyon kapalı, logo/duvar kağıdı kapalı, `layers_hog_keyboard_focus`, otomatik yeniden
  yükleme kapalı.

## 2. Akış (greetd IPC)

İletiler: 4 bayt yerel bayt sıralı uzunluk + JSON; yanıt ≤ 64 KiB, metinler ≤ 512 bayt ve kontrol
karakterinden arındırılmış (`protocol.rs`). Protokol katı istek-yanıttır; tek istek uçuşta.

```text
SelectUser(u) ─▶ create_session(u)
   auth_message secret/visible ─▶ Prompt (alan açık; yer tutucu PAM istemi)
   auth_message info/error     ─▶ mesaj listesine ekle, response:null ile onayla (Busy)
   success                     ─▶ start_session(cmd, env) ─▶ success ─▶ çık(0)
   error auth_error            ─▶ sars, "Yanlış şifre" (ya da PAM'in kendi mesajı), gecikme,
                                  cancel_session ─▶ create_session(u) (taze istem)
   error error                 ─▶ Stopped (genel hata; açıklama yalnızca log'a); Enter yeniden dener
bağlantı koptu                 ─▶ Stopped ("Kimlik doğrulama servisine ulaşılamadı"); Enter: yeniden bağlan
```

- Kullanıcı değişimi uçuştaki yanıtı bekler; o yanıt (başarı dahil) yok sayılır, oturum iptal
  edilir, yeni kullanıcı için açılır — eski kullanıcının "başarı"sı asla oturum başlatmaz (test).
- Artan gecikme kilitle aynı (`backoff_for`: 3 serbest, sonra 2/4/8/16/30 sn); sistem
  `pam_faillock` ayrıca uygulanır, mesajı alanın altında görünür.
- greetd'nin `auth_error` açıklaması (`pam_authenticate: AUTH_ERR`) gösterilmez.

## 3. Kullanıcılar, oturumlar, son seçim

- Kullanıcılar `/etc/passwd` (≤ 1 MiB) + `/etc/login.defs` `UID_MIN..UID_MAX` (varsayılan
  1000..60000); kabuğu `nologin`/`false` olanlar ve geçersiz adlar gizli; ≤ 64. Liste boşsa ad
  elle yazılır. Birden çok kullanıcıda avatarın iki yanında oklar, klavyede Yukarı/Aşağı.
- Görünen ad GECOS'un ilk alanı. Avatar `/var/lib/AccountsService/icons/<ad>` (greeter evleri
  okuyamaz), kilitle aynı yalıtılmış çözücüde.
- Oturumlar `$XDG_DATA_DIRS/wayland-sessions/*.desktop` (dosya ≤ 16 KiB, ≤ 32 oturum; `Hidden`,
  `NoDisplay` gizli). `Exec` Desktop Entry kurallarıyla bölünür, alan kodları atılır, kabuk yok.
  Ortam: `XDG_SESSION_TYPE=wayland`, `XDG_SESSION_DESKTOP=<kimlik>`, `XDG_CURRENT_DESKTOP=<DesktopNames>`.
  `nuxut.desktop` (`dist/session/`; `Exec=/usr/lib/nuxut/nuxut-session`) listede en başta ve varsayılan.
  Sol alttaki düğme sıradaki oturuma geçer.
- Son kullanıcı/oturum `/var/lib/nuxut-greeter/state/state.toml` (greeter'ın, 0700; yalnızca
  listede varsa kullanılır). Yazılamazsa sessizce geçilir.

## 4. Klavye düzeni

Şifre doğru düzende yazılmalı. Başlatıcı `/etc/X11/xorg.conf.d/00-keyboard.conf`'tan
(`localectl set-x11-keymap`) `XkbLayout`/`XkbVariant`/`XkbOptions` okur, değer yalnızca
`[a-z0-9_,:-]` ise `input.lua`'ya yazar (Lua enjeksiyonu yok), yoksa `us`. Düzen listesi
`NUXUT_GREETER_KB_LAYOUTS` ile giriş ekranına geçer (`keyboard.rs`: yeniden doğrulanır, en çok 4).
Sağ altta, güç düğmelerinin solunda etkin düzen (`TR`) görünür; birden çok düzen varsa tıklama ya
da Enter sıradakine geçer (`hyprctl switchxkblayout all <dizin>`, ayrı thread). Sistem düzenini
kurulum yazar: yardımcının `greeter apply --layout … [--variant …] [--options …]` → `localectl` (§9).

## 5. Görünüm: sisteme yayımlanan tema

Greeter kullanıcının ev dizinine erişemez. Yardımcının `greeter apply`'ı
çağıranın (`PKEXEC_UID`; ev dizini `passwd`'den, argüman olarak yol alınmaz) şu dosyalarını `/var/lib/nuxut-greeter/config/nuxut/` altına
yayımlar (root, 0644; dizin 0755). Başlatıcı `XDG_CONFIG_HOME` ve `XDG_STATE_HOME`'u bu kopyaya
çevirir.

| Dosya | İçerik |
|---|---|
| `framework.toml` | Tema kipi, vurgu, yazı ölçeği, `[locale]` (aynen; TOML olarak doğrulanır) |
| `palette.toml` | Üretilmiş palet; `[meta] image` kopyanın yoluna çevrilir (dosya yoksa silinir) |
| `lock.toml` | Kilit ekranı ayarları; dosya duvar kağıdı (`wallpaper`) çıkarılır, masaüstününki geçer |
| `shell.toml` | Yalnızca `[wallpaper] path` (yeniden yazılmış) |
| `wallpaper.<uzantı>` | Duvar kağıdı kopyası (png/jpg/webp/jxl/avif/svg; yerleşik `nuxut:<id>` ise dosya yok) |
| `greeter.toml` | `screen_reader = true/false` (§7) |

Kullanıcı dosyaları yalnızca **çağıranın sahip olduğu düz dosyalarsa** okunur: son bileşen
sembolik bağ olamaz (`O_NOFOLLOW` + ön denetim), açılan dosyanın sahibi çağıran olmalı (ara dizinden
bağ başka bir yere yönlendirse bile sahibi tutmaz → root'un ya da başkasının dosyası sızmaz).
Sınırlar: TOML ≤ 256 KiB, görsel ≤ 32 MiB. Yeni yayın önce `nuxut.new`'e yazılır, sonra yerine
konur (yarım yayın olmaz). Hiç yayımlanmamışsa framework varsayılan teması ve yerleşik varsayılan
duvar kağıdı: yine Nuxut görünümü. Tema değişince yeniden yayımlama: Ayarlar'da "Giriş ekranına da
uygula" (Ayarlar işi; `login-screen apply` yeniden yayımlar, kayıt korunur).

## 6. Parmak izi

greetd PAM'i kendi yığınıyla (`/etc/pam.d/greetd`) yapar. Yığında `pam_fprintd` varsa greeter onun
bilgi mesajını ("Parmağınızı okuyucuya koyun") alanın altında gösterir, alan o sırada kapalıdır
(protokol araya girmeye izin vermez; modül zaman aşımından sonra şifre istemi gelir). Ayrıca parmak
iziyle girişte anahtarlık (gnome-keyring/kwallet) açılmaz. Bu yüzden **giriş varsayılanı şifredir**;
Nuxut kurulumu greetd PAM'ine `pam_fprintd` eklemez. Kilit ekranı parmak izini fprintd D-Bus ile
şifreyle paralel yapar (değişmedi).

## 7. Güç ve erişilebilirlik

- Kapat / yeniden başlat: logind `PowerOff`/`Reboot` (etkileşimsiz). Düğmeler yalnızca
  `CanPowerOff`/`CanReboot` = `yes` ise görünür (`challenge` greeter'da soramaz).
- Klavye: Tab alan → oturum → klavye düzeni → yeniden başlat → kapat; Enter/Boşluk etkinleştirir;
  Ctrl+U ve Esc alanı temizler; birden çok kullanıcıda Yukarı/Aşağı.
- Ekran okuyucu: greeter framework'ün `a11y` özelliğiyle derlenir (AT-SPI ağacı; ADR-0011).
  Yayımlanan `greeter.toml`'da `screen_reader = true` ve `/usr/bin/orca` varsa başlatıcı
  `a11y.lua`'ya orca'yı başlatan satırı yazar; AT-SPI veri yolu greeter oturumunun D-Bus'ından
  etkinleşir. Açmak: `nuxut-shell login-screen apply --screen-reader`.
- Büyük yazı ve yüksek karşıtlık: yayımlanan `framework.toml` ile gelir.
- Ekran klavyesi (dokunmatik): greeter'da kabuk yok; açık iş (§12).

## 8. Tehdit modeli

**Korunan:** kullanıcıların şifreleri ve hesapları; greeter'dan oturum dışına kaçış.
**Saldırgan:** klavye/fare/dokunmatik başındaki kişi; greetd'den gelen beklenmedik yanıtlar;
paketlerle gelen `.desktop` dosyaları ve kullanıcıların koyduğu avatar görselleri.

- **Ayrıcalık:** greeter `greeter` kullanıcısıyla çalışır; PAM greetd'de (root). Greeter şifreyi
  doğrulayamaz, yalnızca iletir; kaba kuvvetin sınırı PAM/faillock + greeter'ın gecikmesi.
  Greeter yeniden başlarsa gecikme sayacı sıfırlanır; asıl sınır faillock'tur.
- **Şifre:** alan (framework `TextInput`, gizli kip; kopyala/yapıştır yok, IME'ye çevre metin
  gitmez) → `SecretBuf` (sabit kapasite, mlock, düşerken sıfırlanır) → yanıt gövdesi
  (`Zeroizing`, baştan en kötü kaçış kapasitesiyle ayrılmış: yeniden tahsis yok) → soket. Log'a,
  IPC'ye (`msg status` yalnızca evre adı), `Debug`'a gitmez. Süreç `PR_SET_DUMPABLE=0`.
- **Dış girdi:** greetd yanıtı ≤ 64 KiB + JSON doğrulama + metin arındırma; `.desktop` ≤ 16 KiB,
  kabuk yorumlanmaz (greetd `exec` eder); `/etc/passwd` sınırlı ve süzülmüş; avatar yalıtılmış
  çözücüde (`LockImages`, duvar kağıdı profili). Hiçbir girdi paniğe yol açmaz.
- **Kiosk kaçışı:** greeter ayarında bind yok; Hyprland'in "acil durum" bind'leri (hiç bind
  yokken ya da Lua hatasında SUPER+Q terminal, SUPER+R çalıştırıcı, SUPER+M çıkış) dosyanın
  **ilk satırındaki** zararsız bind ile kapalı. `hyprctl` soketi greeter'ın çalışma dizininde.
  VT değiştirme (Ctrl+Alt+F*) açık kalır (kurtarma için).
- **Kök yardımcı (`greeter` alanı):** ADR-0008 ilkeleri — sabit yol ve alan başına polkit eylemi,
  sınırlı argümanlar (klavye `[a-z0-9_,:-]`; yol ya da içerik yok), beyaz listeli yazma ve
  doğrulanmış dizin tanıtıcıları, yedek + günlük, hata = geri dönüş, dış komutlar mutlak yolla ve
  temiz ortamla. Kullanıcı dosyaları yalnız çağıranın sahip olduğu düz dosyalarsa (§5). greetd
  ayarı ikiliye gömülü sabit içerik.
- **Kapsam dışı:** root, greetd'nin kendisi, çekirdek; fiziksel erişimle disk.

## 9. Kurulum ve geri alma

Sistem değişikliği yalnız kullanıcı onayıyla, kök yetkili sistem yardımcısının **`greeter`
alanıyla** yapılır (ADR-0008: tek kök ikili `/usr/lib/nuxut/nuxut-system-helper`, alan başına
polkit eylemi). Kod: `crates/nuxut-system-helper/src/greeter.rs`; kullanıcı tarafı
`nuxut-shell login-screen status|apply|revert` (`src/services/login_screen.rs`).

- **Hoş geldin** → "Sistem görünümünü Nuxut'a geçir" (seçili gelir): önyükleme temasıyla birlikte
  `nuxut-shell login-screen apply --json` → `pkexec /usr/lib/nuxut/nuxut-system-helper greeter
  apply --layout <çalışan Hyprland'in düzeni> …`; polkit eylemi `org.nuxut.system-helper.greeter`
  (`exec.path` + `exec.argv1 = greeter`, `auth_admin`). Durum: `login-screen status --json` (kök
  gerekmez): `{"status":{greetd_installed, greeter_installed, display_manager, previous, active},
  "helper":"installed|missing|unsafe", "helper_installable":bool}`; çıkış kodu 3 = yönetici onayı
  verilmedi. Ayarlar adımı açılış görünümüyle aynı adımda ("Sistem görünümü", akış sürümü 3):
  seçilenleri tek "Uygula" `nuxut-shell system-look apply --json` ile tek yönetici onayıyla
  uygular (yardımcının `system-look` alanı, ADR-0008 ek); sonuç ve "Geri al" adımda (2026-10-17).
  Yardımcı kurulu değilse uygulama önce hazırlanmış dosyaları yönetici onayıyla kurar
  (`system-theme install-helper`, ADR-0008 ek); kullanıcıya komut yazdırılmaz.
- **`sh install.sh --system-theme`**: yardımcı, polkit eylemi ve giriş ekranı dosyaları hazırlanır
  (normal kurulum da hazırlar, kök dosya sistemine dokunmaz), gerekirse `pacman -S greetd`, sonra
  önyükleme teması ve `login-screen apply` (pkexec). `--system-theme-revert` ikisini de geri alır;
  `--uninstall` giriş ekranı hâlâ Nuxut'sa hatırlatır.

Kurulan dosyalar (hepsi kökün; yardımcı başkasınca yazılabilen ya da bağ olanı reddeder):
`/usr/bin/nuxut-greeter`, `/usr/lib/nuxut/{nuxut-greeter-session,nuxut-session}`,
`/usr/share/nuxut/greeter/hyprland.lua`, `/usr/lib/tmpfiles.d/nuxut-greeter.conf`,
`/usr/share/wayland-sessions/nuxut.desktop`. Kullanıcıdan hiçbir çalıştırılabilir dosya sisteme
kopyalanmaz.

`greeter apply` sırası (beyaz liste: `etc/greetd/config.toml` + yedekleri, kayıt, yayın dizini):

1. Ön koşul: `greetd` (`/usr/bin/greetd`, `greeter` kullanıcısı) ve yukarıdaki dosyalar. Eksikse
   hiçbir şey yazılmaz (`greetd-missing`, `greeter-user-missing`, `untrusted-file:<yol>`).
2. Kayıt (`/var/lib/nuxut/greeter/state.json`, ilk kayıt kazanır): önceki display manager
   (`display-manager.service` bağının hedefi; yoksa `tty`), greetd ayarı vardı mı, Nuxut'unkinden
   farklıysa yedeği (`config.toml.nuxut-<zaman>.bak`).
3. `dist/greeter/greetd-config.toml` (ikiliye gömülü sabit içerik) → `/etc/greetd/config.toml`;
   `systemd-tmpfiles --create`.
4. Görünüm yayımı (§5).
5. Önceki DM varsa `systemctl disable <dm>`, sonra `systemctl enable greetd.service`.
6. `localectl set-x11-keymap` (başarısızlık yalnız uyarı).

Herhangi bir adım başarısızsa bu turun dosya değişiklikleri geri alınır (`RolledBack`); önceki DM
kapatıldıysa yeniden etkinleştirilir. Etkisi bir sonraki açılışta; çalışan oturuma dokunulmaz.

**Geri alma** (`greeter revert`): `systemctl disable greetd`; önceki bir DM ise onu yeniden
etkinleştirir, `tty` ise hiçbirini (tty + `start-hyprland` düzeni); greetd ayarının yedeği geri
konur (önceden yoksa Nuxut'unki silinir); kayıt silinir. Nuxut dosyaları kalır, zararsızdır.
Olaylar dil bağımsız JSON (`login-screen`, `login-step`, `login-result`, `done`).

## 10. Nasıl denenir (VM, kullanıcı)

```sh
sh install.sh --system-theme        # greetd + giriş ekranı (sonraki açılışta etkin), önyükleme teması
nuxut-shell login-screen status     # durum (kök gerekmez)
# Açılışı değiştirmeden yalnız denemek: boş bir VT'de (çalışan oturumun VT'si değil)
sed 's/^vt = 1/vt = "next"/' /etc/greetd/config.toml > /tmp/greetd-deneme.toml
sudo greetd --config /tmp/greetd-deneme.toml        # Ctrl+Alt+F2'den; Ctrl+C ile durur
```

Geri al: `sh install.sh --system-theme-revert` ya da `nuxut-shell login-screen revert`.

## 11. Test

- Birim: `protocol` (şema, kaçış, kapasite, boyut sınırı, kesik çerçeve), `client` (gidiş-dönüş,
  koparma + yeniden bağlanma, büyük yanıt, soket yok), `flow` (mutlu yol, şifresiz hesap, kimlik
  hatası → iptal → yeni oturum, faillock mesajı, gecikme, bilgi mesajları, kullanıcı değişimi
  yarışları, bağlantı kopması, başlatma hatası, istenmemiş yanıt), `sessions`, `users`,
  `remember`, `power`, `view` (yer tutucu, satırlar), `cli`, `keyboard` (doğrulama, döngü,
  enjeksiyon), i18n kataloğu.
- Kök yardımcı `greeter` alanı (`crates/nuxut-system-helper/src/greeter/tests.rs`, geçici kök +
  sahte systemctl/localectl/systemd-tmpfiles): tty ve sddm'den apply + revert, ikinci apply kaydı
  ezmez, ön koşullar yazmadan önce (başkasınca yazılabilen/bağ olan dosya reddi), greetd
  etkinleştirilemezse geri dönüş ve önceki DM yeniden açılır, localectl argümanları, argüman
  ayrıştırma, görünüm yayımı (sahip/bağ/boyut denetimi, yol yeniden yazımı, yeniden yayında eski
  dosya kalmaz, ayar dosyası bağsa tüm tur geri alınır). Kullanıcı tarafı `login_screen`: durum.
- Entegrasyon (`tests/greetd_flow.rs`): durum makinesi + soket thread'i + sahte greetd
  (`tests/common/fake_greetd.rs`: şifre, parmak izi bilgisi + şifre, iki adımlı kod, kilitli hesap,
  kullanıcı değişimi).
- Canlı (`greeter/tools/live/run.sh`): yalıtılmış headless sway içinde, sahte greetd soketiyle
  (`examples/fake_greetd.rs`), test passwd/oturum dizinleriyle (`NUXUT_GREETER_INSTANCE` tanımlıyken
  okunur); iki çıkış, yanlış şifre, kilitli hesap, kullanıcı değişimi, klavye düzeni düğmesi
  (`NUXUT_GREETER_KB_LAYOUTS=tr,us`, TR → US), test güç düğmeleri (`NUXUT_GREETER_POWER`, yalnız
  test kipinde; basılınca yalnız loglanır), doğru şifre → `nuxut-session` başlatılır. Ekran
  görüntüsü `grim -o HEADLESS-1/2`. Gerçek kimlik doğrulama yok.

## 12. Açık işler

- Ekran klavyesi greeter oturumunda (kabuk yok; OSK'nın ayrı süreç biçimi).
- cage/sway (katman yüzeyi olmayan) ortam için tek pencere kipi.
- Ayarlar: "Giriş ekranı" bölümü (yeniden yayımla, geri al, ekran okuyucu; Hoş geldin adımı var);
  avatarın AccountsService'e de yazılması.
- Kullanıcı listesi görünümü (oklar yerine sıra).

