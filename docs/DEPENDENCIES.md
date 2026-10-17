| `udisks2` (+ `xdg-utils`; `ntfs-3g`/`exfatprogs` yabancı dosya sistemleri için) | Çıkarılabilir medya: takılınca "Aç / Çıkar" bildirimi, bağla/ayır/güç kes (`services::removable_media`; D-Bus ile etkinleşir). Açmak `xdg-open` ile | Bildirim çıkmaz, `msg removable list` `available: false` |
# nuxut-shell — Bağımlılıklar

Kural: kod Rust'tır; doğru, bakımlı ve hıza/estetiğe zarar vermeyen crate'ler ve
sistem kütüphaneleri kullanılır. Her yeni crate ve sistem paketi buraya gerekçe, alternatif,
lisans ve çalışma zamanı paketi (Arch adıyla) ile yazılır.
Proje lisansı: MIT — bağımlılıkların hepsi MIT ile uyumlu (MIT /
Apache-2.0 çift lisans).

## Çalışma zamanı: zorunlu sistem paketleri ve servisler (Arch)

| Paket | Neden |
|---|---|
| `hyprland` | Compositor; IPC soketleri (`hyprctl` yalnızca geliştirme/test) |
| `mesa` (EGL/GLES 3.0), `wayland`, `libxkbcommon` | Framework runtime (nuxut-wayland) |
| `ttf-material-symbols-variable` | Bar/panel ikonları (Material Symbols Rounded, FILL ekseni) |
| `dbus` | Oturum bus'ı (tepsi, bildirim) |
| `systemd` (systemd-logind) | Oturum eylemleri: kilit, oturumu kapat, askıya al, yeniden başlat, kapat |

## Çalışma zamanı: opsiyonel (yoksa ilgili özellik sessizce devre dışı)

| Paket | Kullanan | Yoksa |
|---|---|---|
| `polkit` (+ bir polkit ajanı, ör. `hyprpolkitagent`) | Askıya al/kapat yetkisi `challenge` dönerse | logind reddeder, düğme hata gösterir |
| `xdg-desktop-portal` (+ `xdg-desktop-portal-hyprland`/`-gtk`) | Bildirimdeki bağlantıyı açmak (OpenURI) | Bağlantı açılmaz, uyarı loglanır |
| `hyprlock` / `hypridle` (ya da logind `Lock` sinyalini dinleyen herhangi bir kilit) | "Kilitle" eylemi logind `Session.Lock` gönderir | Kilit ekranı gelmez |
| `nuxut-calendar` (nuxut-calendard, `org.nuxut.Calendar1`) | Saat/takvim panelindeki etkinlikler | Takvim boş görünür |
| `xembedsniproxy` (plasma-workspace) | Yalnızca eski X11 XEmbed tepsi uygulamaları | Bu uygulamaların ikonu görünmez |
| `networkmanager` | Ağ servisi (durum, Wi-Fi, kayıtlı bağlantılar, VPN, şifre ajanı) | Ağ bölümü "kullanılamıyor" |
| `bluez` | Bluetooth servisi (adaptör, cihazlar, eşleştirme ajanı) | Bluetooth bölümü "kullanılamıyor" |
| `upower` | Pil yüzdesi, şarj durumu, kalan süre, AC | Pil göstergesi yok |
| `power-profiles-daemon` (ya da `tuned-ppd`) | Güç profili (performans/dengeli/tasarruf) | Profil seçici yok |
| `vimix-cursors`, `papirus-icon-theme` (install.sh varsayılan kurar) | Birleşik görünümün önerilen imleci (Vimix, kullanıcı kararı 2026-10-11) ve ikon teması (Papirus) | İmleç ve ikon Adwaita'ya düşer |
| `capitaine-cursors`; AUR `material-cursors-git`, `oreo-cursors-bin`, `volantes-cursors`, `qogir-cursor-theme`, `sweet-cursors-git` (install.sh `--with-cursors`) | Ayarlar'daki imleç seçiminde ek seçenekler (kurulu temalar taranır) | Listede görünmezler. Future imleçleri depoda/AUR'da yok |
| `ddcutil` (yalnızca udev kuralı: `/dev/i2c-*` için `uaccess`) ve `i2c-dev` çekirdek modülü | Harici monitör parlaklığı (DDC/CI, `/dev/i2c-N` doğrudan; ddcutil ikilisi çağrılmaz) | Yalnızca dahili arka ışık |
| `pipewire`, `wireplumber` (`libpipewire`) | Ses servisi: cihazlar, seviye, sessiz, uygulama akışları. Seviye/sessiz YAZIMI `wpctl` (wireplumber) ile — GNOME/KDE/pavucontrol ile aynı ölçek ve yol (kart route, BT mutlak ses); `wpctl` yoksa doğrudan PipeWire yazımı (geçici hotfix, bkz. 2026-10-07 notu) | Ses bölümü "kullanılamıyor" |
| `ttf-nerd-fonts-symbols` ("Symbols Nerd Font"; yoksa temanın `mono` ailesi bir Nerd Font ise o, ör. `ttf-jetbrains-mono-nerd`) | Workspace göstergesinin `icons` stilinde uygulama glifleri (`nf-md/linux/dev/fa`, tablo framework `nuxut-ui/src/app_glyphs.txt`). Alternatif: uygulamanın kendi ikon teması (renkli doku) — bar boyunda tek renkli glif daha sakin ve ikon temasına bağımlı değil | Numara gösterilir |
| `grim` | Ekran görüntüsü yakalama (`msg shot`; compositor'ın wlr-screencopy / ext-image-copy-capture'ı; PPM stdout). Alternatif: framework'te `ext-image-copy-capture-v1` istemcisi (ileride), `hyprshot`/`grimblast` (kabuk betiği, yine grim) | `shot` hata döner, bildirim "Ekran görüntüsü alınamadı" |
| `wl-clipboard` | Ekran görüntüsünü panoya (`wl-copy --type image/png`). Alternatif: framework panosuna ikili MIME (şu an yalnızca metin) | Yalnızca kaydedilir, yanıtta `warning` |
| `xdg-utils` | Bildirimdeki "Aç" (`xdg-open`) ve FileManager1 yoksa "Klasörde göster" | Düğme bir şey açmaz, uyarı loglanır |
| `noto-fonts-emoji` (Noto Color Emoji; ya da `ttf-twemoji`) | Başlatıcının emoji kipinde renkli glifler (framework renkli glif atlası; fontconfig yedeği). Veri Emoji 16.0 ile sınırlı: bu fontların kapsadığı en yeni sürüm (docs/design/emoji.md) | Emoji kutu (tofu) ya da tek renkli çizilir; arama ve panoya kopyalama çalışır |

## Crate'ler — çekirdek

| Crate | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `nuxut*` (path) | MIT | Framework: geometri, yay, config, tema, render, runtime | — |
| `nuxut-calendar-ipc` (path, `../calendar`) | MIT | Takvim daemon'unun D-Bus sözleşmesi (tipler + proxy; yalnızca zbus + serde, ağ/DB yok) | Arayüzü shell'de kopyalamak: sözleşme iki yerde, kayar |
| `serde`, `serde_json` | MIT/Apache-2.0 | Hyprland `j/` JSON'u, toleranslı serde | Elle ayrıştırma: hataya açık |
| `toml` | MIT/Apache-2.0 | `[bar.widget.<ad>]` tablolarını ham okuyup widget kaydından çözmek; nuxut-config ile aynı sürüm (ek derleme maliyeti yok) | Her widget için sabit alan: kaydı tek yerde tutmayı bozar |
| `chrono` (`clock`, `std`, `unstable-locales`; varsayılan özellikler kapalı) | MIT/Apache-2.0 | strftime doğrulama/biçimlendirme, saat dilimi; `unstable-locales` yerelleştirilmiş ay/gün adları içindir (framework `nuxut-locale` zaten açıyor, yeni crate yok) | `time`: strftime biçim dili yok; `jiff`: biçim dili farklı |
| `thiserror` | MIT/Apache-2.0 | Hata tipleri | Elle `Display` |
| `tracing`, `tracing-subscriber` (fmt, env-filter) | MIT | Log; framework ile aynı | `log` + `env_logger`: framework tracing kullanıyor |
| `unicode-segmentation` 1.13 | MIT/Apache-2.0 | Etkin pencere başlığını grafem sınırında kırpma (`max_chars`; birleşik emoji/aksan bölünmez). cosmic-text (framework) üzerinden zaten ağaçta: ek derleme maliyeti yok, saf Rust, çalışma zamanı paketi yok | `chars()` ile kırpma: ZWJ emoji ve birleşik aksanı ortadan böler |
| `zbus` 5 (varsayılan async-io) | MIT | Saf Rust D-Bus: logind, StatusNotifier/DBusMenu, bildirim sunucusu; libdbus'a C bağımlılığı yok | `dbus` crate (libdbus FFI): C bağımlılığı |
| `blocking` =1.6.2 (yalnız sürüm sabiti; zbus'ın dolaylı bağımlılığı) | Apache-2.0/MIT | zbus'ın iş parçacığı havuzu. 1.7.0'da boştaki havuz thread'leri hiç çıkmıyor ve 500 ms'de bir uyanıyor (shell boşta 2–3 thread × 2/sn; ölçüm `docs/research/perf-2026-10.md`); 1.6.2'de 500 ms boşta kalan thread çıkar. Geçici: kök neden ve kaldırma koşulu "Sürüm sabitleri"nde | Sabitsiz 1.7.0: süreç ömrü boyunca boşta uyanma |

## Crate'ler — tepsi

| Crate | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `async-channel`, `async-executor`, `async-io`, `futures-lite` | Apache-2.0/MIT | zbus'un zaten getirdiği geçişli bağımlılıklar; tepsi thread'inde tek iş parçacıklı yürütücü, komut kuyruğu ve zamanlayıcı. Ek derleme maliyeti yok, saf Rust | `tokio`: ağır, ikinci bir çalışma zamanı |

## Crate'ler — bildirimler

| Crate | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `url` 2 | MIT/Apache-2.0 | Gövde `<a href>` bağlantılarını WHATWG ayrıştırıcıyla doğrulamak ve normalleştirmek (şema allowlist, host, userinfo, IDN → punycode). Güvenlik incelemesi gereği | Elle şema/host ayrıştırma: kenar durumlarda atlatılabilir |
| `rustix` 1.1 (`fs`) | Apache-2.0 (LLVM istisnalı)/Apache-2.0/MIT | `image-path`'i `O_NOFOLLOW\|O_NONBLOCK` ile bir kez açıp `fstat`/`fstatfs` — `unsafe` olmadan; zbus/async-io üzerinden zaten ağaçta | `libc` + `unsafe` (crate `unsafe_code = forbid`); `nix`: ek bağımlılık |

## Crate'ler — sistem servisleri

| Crate | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `pipewire` 0.10 (pipewire-rs, `libpipewire-0.3` binding'i) | MIT | Ses: PipeWire registry'sini doğrudan dinlemek (düğüm/cihaz/metadata, `Props` parametreleri) — olay tabanlı, alt süreç yok. `unsafe` crate içinde; shell `unsafe_code = forbid` korunur | `wpctl`/`pw-dump` alt süreci: ayrıştırma + süreç maliyeti, olay yok; `libpulse-binding`: pipewire-pulse katmanı üzerinden dolaylı |
| `i2cdev` 0.6 (rust-embedded) | MIT/Apache-2.0 | DDC/CI için `/dev/i2c-N` üzerinde `I2C_SLAVE` ioctl'u — `unsafe` olmadan | `ddcutil` alt süreci: monitör başına saniyeler, süreç maliyeti; `ddc-i2c`: bakımsız |
| `zeroize` 1 | MIT/Apache-2.0 | Wi-Fi şifresi bellekte kısa süre: düşürülünce sıfırlanır | Elle sıfırlama: derleyici optimizasyonuyla atlanabilir |
| `nuxut-net` (path, framework; ureq + rustls/ring + sistem kökleri) | MIT | Oynatıcının verdiği `https` kapak adresini indirmek: `fetch_untrusted_media` yalnızca genel internete gider (özel ağ, loopback, link-local, metadata adresleri reddedilir; yönlendirmeler dahil), yalnızca https, 8 MiB / 5 sn sınırlı. SSRF denetimi framework'te tek yerde | Shell'de doğrudan `ureq`: SSRF denetimi ikinci kez yazılır ve kayar (önceki sürüm bu yüzden kaldırıldı); ağ kapaklarını hiç göstermemek |

**İç crate'ler (2026-10-13, ağ planı aşama 1):** `crates/nuxut-dbus`, `crates/nuxut-nm`,
`crates/nuxut-bluez` — kabuğun D-Bus altyapısı ile NetworkManager/BlueZ istemcilerinin yalnız
taşınmış hâli (docs/design/network-settings.md). **Yeni dış crate ya da sistem paketi yok:**
yalnızca kabuğun zaten kullandığı `zbus` 5.16, `async-channel`/`async-executor`/`async-io`/
`futures-lite`, `tracing`, `zeroize` (aynı sürümler, ağaçta tek kopya). Framework'e bağlı değiller.

**İç crate `crates/nuxut-power` (2026-10-15, t117 veri yolu B):** kabuğun güç servisinin
(UPower + power-profiles-daemon) Ayarlar ile paylaşılmak üzere taşınmış hâli. Yeni dış
bağımlılık yok: `zbus` 5.16, `async-channel`, `futures-lite`, `tracing`, `serde_json` 1
(power-v1 JSON'u); kabukla aynı sürümler. Framework'e bağlı değil; i18n yok (hatalar `PowerError`).

## Crate'ler — görsel yalıtımı (framework ADR-0007)

| Crate | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `nuxut-image-sandbox` (path, framework; ana süreç tarafı: rustix 1, libc sabitleri, blake3, `nuxut-image`; seccompiler/landlock/`jxl` yalnızca framework'teki ayrı `nuxut-image-worker` paketinde) | MIT (bağımlılıklar Apache-2.0/MIT/BSD-3/CC0) | Tepsi, bildirim ve MPRIS kapağının güvenilmeyen baytlarını ayrı, kısıtlı süreçte çözmek: çözücü paniği/bellek hatası/sonsuz döngü shell'i (`panic = "abort"`, parola girişi, pano) düşürmez; SVG ikonlar artık görünür. Shell'de yalnızca `Sandbox` (havuz, hız sınırı, negatif önbellek); işçi ikilisi framework paketi (`cargo build -p nuxut-image-worker --manifest-path ../framework/Cargo.toml`). Sistem paketi gerekmez; kurulum: `~/.local/lib/nuxut/nuxut-image-worker` (install.sh). Ayrı paket sayesinde `jxl-oxide` kabuk ağacında yok (`cargo tree -i jxl-oxide` boş); güvenilmeyen JXL yalnızca işçide çözülür (işçi yoksa K2-b JXL'yi süreç içine düşürmez) | Süreç içi çözüm (bugünkü güven kapısı: SVG reddi, raster süreç içinde — panik/bellek hatası shell'in); `/proc/self/exe` yeniden çalıştırma (ADR-0007'de reddedildi: tek-örnek IPC/config kancadan önce çalışır, işçi tüm dinamik kütüphaneleri eşler) |

## Crate'ler — dış yapılandırma yazımı

| Crate | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `nuxut-apply` (path, framework; ek çalışma zamanı paketi yok — dconf/systemctl yalnızca Ayarlar'ın görünüm işlemlerinde, shell bugün onları yazmaz) | MIT | ADR-0001: shell.toml göçü, Hyprland `source` satırı, setup.toml, Ekranlar dosyaları tek atomik yazıcı + ortam kapısından (test örneği kullanıcının dosyalarına asla yazmaz); `hypr.source`/`migration.shell` snapshot'ı ve `msg snapshot` geri yüklemesi. Ayarlar ile aynı depo ve kilit | Shell'deki eski yazıcılar (`AtomicFileWriter`, `hypr_config::writer`, setup'ın kendi yazıcısı): üç ayrı atomik yazıcı, `.bak` yedekleri, snapshot yok; ADR-0001 bu yüzden framework'e taşıdı |

## Crate'ler — başlatıcı

| Crate | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `nucleo-matcher` 0.3 (`unicode-casefold`, `unicode-normalization`; `unicode-segmentation` kapalı) | MPL-2.0 (dosya düzeyinde copyleft; değiştirmeden bağımlılık olarak kullanmak MIT projeyle uyumlu) | helix/nucleo'nun eşleştiricisi: fzf puanlaması, Latin aksan normalleştirme, sıfır ayırmalı `Utf32String` samanlıkları; 5000 uygulamada tuş başına < 2 ms. İndeksler karakter (grafem değil) — vurgu için yeterli, bir bağımlılık az | `fuzzy-matcher` (skim; bakımı durdu, daha yavaş), `frizbee` (SIMD; yeni, API oynak), `sublime_fuzzy` (normalleştirme yok) |
| Gömülü veri: `src/services/launcher/data/emoji.tsv` (yeni crate yok, 2026-10-13) | Unicode License v3 (`data/LICENSE-UNICODE`) | Emoji kipinin verisi: 1914 taban emoji, İngilizce + Türkçe CLDR adı/anahtar sözcükleri, emoji-test sırası. `include_str!` ile derlemeye gömülü (~249 KB salt okunur; çalışma zamanında dosya/sistem bağımlılığı yok). Bir kez `examples/emoji_import.rs` (Rust) ile ibus 1.5.34'ün `emoji-en.dict`/`emoji-tr.dict` dosyalarından (CLDR türevi) üretildi; derleme sisteme bağlı değil. Arama katlaması `nucleo-matcher`'ın tabloları (yukarıda) | `emojis` crate (yalnız İngilizce ad + gemoji kısa adları; Türkçe yok, yine ayrı tr verisi gerekirdi), `emoji` crate (CLDR yerelleri özellik bayraklı ama bakımı durmuş, eski Emoji sürümü), CLDR `annotations/tr.xml` + `emoji-test.txt`'yi doğrudan satıcı verisi yapmak (ağ erişimi yoktu; ibus sözlükleri aynı CLDR verisini taşıyor) |
| `rustix` (+`event` özelliği) | — (zaten ağaçta) | inotify (`fs`), `poll` + `eventfd` (servis thread'ini zamanlayıcısız uyutup temiz durdurmak) | `inotify` crate: ek bağımlılık, aynı sistem çağrıları |
| `rustix` (+`net` özelliği, 2026-10-16) | — (zaten ağaçta) | VM pano köprüsü: X sunucusu soketinin karşı ucunun kimliği (`SO_PEERCRED`); başka kullanıcının X sunucusuna pano verilmez | `libc::getsockopt` + `unsafe` |
| `rustix` (+`param` özelliği) | — (zaten ağaçta) | sistem izleyicisi: `clock_ticks_per_second` (süreç CPU payı) ve `page_size` (RSS); `statvfs` (`fs`) disk doluluğu | sabit 100 Hz / 4 KiB varsaymak: bazı mimarilerde yanlış |

## Crate'ler — ekran görüntüsü

| Crate | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `png` 0.18 | MIT/Apache-2.0 | Ekran görüntüsünü PNG'ye kodlama (`Compression::Fast`: fdeflate, 4K'da onlarca ms); `image` üzerinden zaten ağaçta, ek derleme yok. Yakalama PPM olarak kendi ~40 satırlık ayrıştırıcımızla okunur (PNG çöz-kodla turu yok) | `image` (aynı `png`'yi sarar, daha geniş API gereksiz), `grim`'e doğrudan PNG yazdırmak (kesim/birleştirme için yine çözmek gerekir) |

## Crate'ler — gece ışığı (t113)

| Crate / paket | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `sunrise` 3.0 (varsayılan `std`; tek bağımlılığı `chrono`, ağaçta) | MIT | Gün batımı/doğumu (NOAA güneş denklemi), saf Rust, panik yok: kutup günü/gecesinde `None` döner. Küçük (~400 satır), ek derleme maliyeti ihmal edilir. Çalışma zamanı paketi yok | Denklemi elle yazmak (~100 satır, aynı formül; test yükü bizde); `suncalc` (daha geniş API — ay evresi vb. — gereksiz) |
| `tzdata` (Arch, core; zaten kurulu) | Kamu malı | Konum yazılmamışsa saat diliminin şehir koordinatı: `/usr/share/zoneinfo/zone1970.tab` (yoksa `zone.tab`). Yoksa güneş zamanlaması 20:00–07:00'ye düşer (uyarıyla) | geoclue (D-Bus konum servisi): v1 dışı, açık nokta |

Compositor tarafı: Hyprland `hyprland-ctm-control-v1` (framework `nuxut-wayland`);
yeni sistem paketi yok, `hyprsunset` gerekmez.

Hesap makinesi için crate **yok**: `calc.rs` küçük özyinelemeli iniş ayrıştırıcı; gerekçe
modülün başında (sınırlar + ondalık virgül; `evalexpr` yeni sürümleri AGPL, `meval`/`fasteval`
bakımsız, libqalculate C++ ve ağır).

Çalışma zamanı (opsiyonel; yoksa yedek yol): `systemd` kullanıcı yöneticisi + `systemd-run`
(uygulama kendi kapsamında; yoksa kapsamsız), `util-linux` (`setsid -f`; yoksa ayrı süreç
grubunda doğrudan başlatma + toplayıcı thread), `xdg-terminal-exec` ya da bir terminal
(`kitty`, `foot`, `alacritty`, … — yalnızca `Terminal=true` uygulamalar için).

## Crate'ler — ekran klavyesi (t125; docs/design/osk.md)

| Crate / paket | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `xkbcommon` 0.8 (`default-features = false`; libxkbcommon bağlaması) | MIT | Gömülü TR-Q/US keymap'inden tuş etiketlerini çıkarmak: ekranda görünen karakter, sanal klavyeye giden aynı keymap'in ürettiğiyle birebir (Türkçe ı/İ, Caps, AltGr, ölü tuşlar elle tablo yazılmadan). Framework runtime'ı (nuxut-wayland) aynı sürümü zaten bağlıyor: ek derleme yok. Çalışma zamanı paketi `libxkbcommon` (zaten zorunlu) | Elle karakter tablosu (iki kaynak: etiket ile çıktı ayrışabilir, Caps/İ kuralları tekrar yazılır); `xkeysym` (yalnızca keysym adları, keymap derlemez) |

Compositor tarafı: `zwp_virtual_keyboard_v1` (framework `nuxut-wayland`, talep);
yeni sistem paketi yok. Gömülü keymap metinleri `assets/osk/*.xkb` xkeyboard-config 2.48'den
(MIT/X11) üretildi; yeniden üretim komutu tasarım belgesinde.

## Crate'ler — VM pano köprüsü (2026-10-16; docs/design/vm-clipboard.md)

| Crate / paket | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `x11rb` 0.13 (`xfixes` + `res` özellikleri — `res`: XRes ile istemcinin süreç kimliği, köprü yetkisi için; varsayılan saf Rust `RustConnection`) | MIT/Apache-2.0 | Xwayland `CLIPBOARD` seçimini olay tabanlı izlemek (XFixes `SelectionNotify`) ve sunmak (`SelectionRequest`, INCR alımı): VM konuk ajanları panoyu yalnız X11 ile konuşur, Hyprland odaksız eşitlemiyor. Saf Rust istemci: libxcb/libX11 bağlaması ve yeni sistem paketi yok; tek görünmez pencere, boşta `poll` dışında iş yok. Bakımlı (psychon/x11rb), winit/arboard/smithay ağaçlarında yaygın | `xcb` crate'i (libxcb FFI; sistem kütüphanesi ister), `x11-clipboard` (x11rb üstünde basit oku/yaz API'si; sahiplik izleme, tembel çok türlü sunum ve döngü önleme yine bizde kalırdı), `xclip`/`xsel` alt süreçleri (kullanıcının betiği: 0,1 sn yoklama, yalnız metin) |

Çalışma zamanı: yeni paket yok. Köprü yalnızca Xwayland açıksa (`DISPLAY`) ve `auto` kipinde bir
VM konuk ajanı (`spice-vdagent`, `open-vm-tools`, `virtualbox-guest-utils`) çalışırken devreye
girer; konuk ajanı paketleri kullanıcının/VM'in seçimidir, kabuk bağımlılığı değildir. Testte `xclip` ve
`xorg-xwayland` (geliştirme).

## Crate'ler — yerleşik duvar kağıtları (`crates/nuxut-wallpapers`; docs/design/wallpapers.md)

Kullanıcı kararı / 2026-10-13. Shell, kilit ekranı ve (path bağımlılığıyla) ayarlar
ortak kullanır. **Yeni derleme yok**: üç crate de framework `nuxut-image` üzerinden ağaçta, aynı
sürümler (`cargo tree -d` değişmedi). Çalışma zamanı sistem paketi yok (SVG'ler ikiliye gömülü).

| Crate | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `resvg` 0.48 (`default-features = false`) | Apache-2.0 OR MIT | Gömülü SVG'leri çıkışın tam fiziksel boyutunda, cover kırpımıyla (tek dönüşüm, fazladan piksel yok) rasterleştirmek. Framework çalışma alanıyla aynı sürüm; metin/raster özellikleri kapalı (SVG'lerde yalnız yol + gradyan). `nuxut_image::render_svg` güvenilmeyen girdi içindir ve `Cover`'da kırpmadan büyük tuval çizer; gömülü kaynakta denetim bütçesine gerek yok | `nuxut_image::render_svg` (+ ayrı kırpma; 4:3/21:9'da %33'e varan boşa çizim), `rsvg` (C/GLib, ek sistem paketi), önceden raster PNG gömmek (çözünürlük başına dosya, ikili büyür, keskinlik kaybı) |
| `png` 0.18 (yalnız dev-dependency) | MIT/Apache-2.0 | Yalnızca `builtin_bench` ölçüm örneğinin çıktı PNG'leri. Ürün yolu PNG yazmaz: kare bellekte kalır ve `Wallpaper::set_image` ile verilir (2026-10-13; önceki disk önbelleği kaldırıldı, docs/design/wallpapers.md madde 7) | — |
| `nuxut-image` (framework, path, isteğe bağlı `frame` özelliği, `default-features = false`) | MIT | `From<Frame> for WallpaperFrame` (kopyasız `Arc<[u8]>`); shell ve kilit açar, ayarlar açmaz. Zaten ağaçta | Dönüşümü her tüketicide yinelemek |
| `nuxut-i18n` (framework, path) | MIT | Görünen adların crate'le birlikte gelen Fluent kataloğu (`wallpapers` alanı); ayarlar aynı adları kendi kataloğuna ekler | Adları her tüketicinin kendi kataloğunda yinelemek |

## Crate'ler — ayarların kendini onarması (`crates/nuxut-config-heal`; docs/design/config-migrations.md)

2026-10-16. Shell ve (path bağımlılığıyla) Ayarlar ortak kullanır. **Yeni derleme yok**: üç crate de
ağaçta, aynı sürümler. Çalışma zamanı sistem paketi yok. Levenshtein için crate eklenmedi (20 satırlık
Damerau-Levenshtein; `strsim` yalnız bunun için ağaca girerdi).

| Crate | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `toml_edit` 0.25 (`parse`, `display`) | MIT/Apache-2.0 | Span'lı ayrıştırma (`Document`): onarım metin düzeyinde, dokunulmayan baytlar aynı; hata konumu (bozuk satır, tekrar eden anahtar) | `toml` ile yeniden yazmak (yorum ve düzen kaybolur) |
| `toml` 1 | MIT/Apache-2.0 | Eşitlik kâhininin ayrıştırması ve tür hatasının konumu (`de::Error::span`); `nuxut-config` ile aynı yol | — |
| `serde_ignored` 0.1 | MIT/Apache-2.0 | Türün tanımadığı anahtarların yolları (`nuxut-config` zaten kullanıyor) | `deny_unknown_fields` (ilk bilinmeyen anahtarda bütün dosya düşer) |

## Crate'ler — config göçü (ADR-0002) ve planlı ADR-0001 sistem paketi

| Crate | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `toml_edit` 0.25 (`parse`, `display`; varsayılanlar kapalı) | MIT/Apache-2.0 | shell.toml sürüm göçü: anahtarı yeniden adlandırırken/silerken yorum, sıra ve bilinmeyen anahtarları korumak. Ayarlar ile aynı sürüm; `toml` 1 ile aynı `toml_parser` ailesi (ağaçta zaten vardı; ek derleme yalnızca `toml_writer`). Yüklemede (açılış, `config reload`) sürümü okumak için bir kez ayrıştırır; düzenleme yalnızca eski sürümlü dosyada. Sıcak yolda (kare, olay) kullanılmaz. Genel göç çalıştırıcısı framework'e (`nuxut-config`) girerse oradan gelir | `toml` (serde) ile yeniden yazmak: yorumlar ve kullanıcı düzeni kaybolur |
| `semver` 1 | MIT/Apache-2.0 | "Güncelleme bitti" bildirimi (`services::shell_state`): `shell-state.toml` `last_version` ile `CARGO_PKG_VERSION`'ı SemVer 2.0 önceliğiyle (ön sürüm, derleme meta verisi kuralları) karşılaştırmak. Ağaçta zaten vardı (Cargo.lock 1.0.28; ek derleme ihmal edilebilir), bağımlılıksız; açılışta bir kez kullanılır. Çalışma zamanı paketi yok | Elle `major.minor.patch` ayrıştırma: ön sürüm sıralaması (`1.0.0-rc.1 < 1.0.0`) ve `+build` yok sayımı yeniden yazılırdı |
| — sistem: `dconf` (Arch `dconf`) — planlı | — | ADR-0001 dconf adaptörü (`dconf read/write/reset` alt süreci); yoksa o hedef "kullanılamıyor" | `gsettings`: "hiç ayarlanmamış" ayrımını vermez, şema ister |

## Crate'ler — birleşik görünüm (ADR-0004)

| Crate | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `nuxut-palette` (path, framework; `material-colors` 0.4; `default-features = false`) | MIT (material-colors: MIT/Apache-2.0) | GTK/Qt için iki kipin tam Material 3 rol kümesi (`Palette::generate`; matugen algoritmasının saf Rust portu). Türetme framework'te tek yerde (ADR-0004 §12-9). Görsel çözme (`nuxut-image` + jxl) framework'ün varsayılan açık `image` özelliğinde (5c912ce); shell onu kapatır (palet yolundan `jxl-oxide` gelmez; ağaçtaki `jxl-oxide` yalnızca görsel işçisinden, bkz. görsel yalıtımı) | `matugen` ikilisi (ayrı süreç + şablon dosyaları), shell'de kopya türetme (yasak) |
| `material-colors` 0.4.2 (`default-features = false`, `std`) | MIT/Apache-2.0 | Palete göre klasör ikonları (`look::folders`, 2026-10-16): Papirus mavisinin tonunu koruyup ton açısını ve renkliliğini vurguya taşımak (HCT). Framework'ün aynı sürümü ve özellikleri; ağaçta zaten vardı (ek derleme yok). Çalışma zamanı paketi yok | Elle OKLCH dönüşümü (Nuxut'un tonal paletleriyle aynı uzay olmazdı), `papirus-folders` (root ister, yalnız sabit renkler) |

Çalışma zamanı paketleri (opsiyonel; yoksa ilgili hedef eski hâlinde kalır, `msg look status`
`missing_packages` listeler, Nuxut paket kurmaz): `adw-gtk-theme`, `xdg-desktop-portal-gtk`,
`plasma-integration` + `plasma5-integration` (yoksa `qt6ct`/`qt5ct` yedeği), `breeze` (yoksa Fusion),
`papirus-icon-theme` (öneri; yoksa Adwaita), `bibata-cursor-theme-bin` (AUR, öneri; yoksa
`adwaita-cursors`). Ayrıntı ve gerekçe: ADR-0004 §10.

### Nuxut Qt stili (`qt-style/`, ADR-0009; C++ istisnası kullanıcı onaylı 2026-10-16)

| Paket | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| derleme: `cmake`, `qt6-base` (Arch; başlıklar paketle gelir) — install.sh `PKG_BUILD` ile `pacman -S --needed` kurar. `qt5-base` kurulmaz: zaten kuruluysa (Qt5 uygulaması var demektir) Qt5 eklentisi de derlenir | BSD-3 / LGPL-3 | `QProxyStyle` eklentisi yalnızca Qt'nin C++ ABI'siyle yazılabilir; Rust bağlamaları (cxx-qt, qmetaobject) `QStyle` türetmeyi desteklemez. Derleme başarısızsa kurulum durmaz: uyarı + günlük (`$CARGO_TARGET_DIR/qt-style-qt<N>.log`), Qt uygulamaları Breeze/Fusion ile sürer; kurulan eklenti kurulum sonunda doğrulanır | Kvantum (SVG temaları paleti izlemez, ADR-0004), Darkly (yalnız AUR, token okumaz) |
| çalışma zamanı | — | Ek paket yok: eklenti Qt uygulamasının kendi Qt'sine yüklenir; `QT_PLUGIN_PATH` shell'den | — |

İsteğe bağlı: `kitty` (kuruluysa Nuxut renkleri + imleç izi yazılır; docs/design/look-gtk.md).

## Portal arka ucu (`xdg-desktop-portal-nuxut`, `portal/`; docs/design/portal.md)

Yeni crate yok: `nuxut-theme` + `nuxut-config` (path, framework; tema yükleyicisi ve inotify
izleyicisi — renderer/Wayland gelmez), `zbus` 5, `rustix` (`event`: iki inotify tanımlayıcısında
`poll(2)`), `tracing`/`tracing-subscriber` — hepsi zaten ağaçta. Alternatif: `nuxut` cephesi
(renderer ve Wayland'i de bağlardı; UI'sız arka uç için gereksiz), `notify` crate'i (framework'ün
izleyicisi varken ikinci yol).

| Paket (Arch) | Lisans | Neden | Yoksa |
|---|---|---|---|
| `xdg-desktop-portal` (≥ 1.22: `.portal` dosyalarını `$XDG_DATA_HOME` altında da arar) | LGPL-2.1+ | Ön uç: uygulamaların `org.freedesktop.portal.Settings`'i; arka ucumuzu D-Bus ile başlatır. install.sh kurar. Alternatif: yok (portal standardı) | Uygulamalar kip/vurgu değişimini canlı görmez (yalnızca dconf/GTK ayarları) |
| `xdg-desktop-portal-gtk` | LGPL-2.1+ | `nuxut-portals.conf`'ta yedek: `Settings`'te bizim bilmediğimiz ad alanları (`org.gnome.desktop.interface` …, look'un dconf'u), kullanıcının Hyprland seçimi başka arka uç demiyorsa `FileChooser` ve diğer arayüzler (türetilen dosya; kaynak yoksa `default=hyprland;gtk`). install.sh kurar | Bu arayüzler hyprland arka ucunda yoksa sağlanmaz |
| `xdg-desktop-portal-hyprland` | BSD-3-Clause | `default` sırasının başı (ekran paylaşımı, ekran görüntüsü, genel kısayollar); Hyprland kurulumuyla gelir. Ekran paylaşımında seçici olarak `nuxut-sharepicker`'ı çağırır (`xdph.conf` `screencopy:custom_picker_binary`; docs/design/screenshare-picker.md) | gtk'ye düşülür; seçici bloğu etkisiz kalır |

## Sistem görünümü (`crates/nuxut-boot-theme`, `crates/nuxut-system-helper`; 2026-10-16, docs/design/boot-theme.md)

Yeni dış crate yok; hepsi ağaçta aynı sürümle.

| Crate | Lisans | Kullanan | Gerekçe / alternatif |
|---|---|---|---|
| `resvg` 0.48 (`default-features = false`) | Apache-2.0/MIT | nuxut-boot-theme | Logo, 9 parçalı kutular, ikonlar ve dönen gösterge karelerini kendi kurduğumuz SVG'lerden çizer (metin/raster özellikleri kapalı). Alternatif elle tiny-skia yolu: aynı bağımlılık, daha çok kod |
| `png` 0.18 | MIT/Apache-2.0 | nuxut-boot-theme | Tema PNG'leri ve önizleme; önizlemede yalnız kendi ürettiğimiz baytları çözer |
| `rustix` 1.1 (`fs`, `process`) | Apache-2.0/MIT | nuxut-system-helper | `openat`/`O_NOFOLLOW`/`renameat`/`unlinkat`/`fstat`/`flock`/`fstatvfs`/`umask`/`geteuid`: kök yetkiyle sembolik bağ ve TOCTOU savunması, `unsafe` olmadan. Alternatif `nix`: ağaçta iki sürümü var, rustix zaten kabuğun seçimi |
| `blake3` 1 (`std`) | CC0/Apache-2.0 | nuxut-system-helper | Bildirimdeki içerik özeti: geri almadan önce dosya bizden sonra değişmiş mi |
| `toml` 1 (ağaçta) | MIT/Apache-2.0 | nuxut-system-helper (`greeter` alanı) | Giriş ekranına yayımlanan kullanıcı görünüm dosyalarını ayrıştırıp doğrulamak, duvar kağıdı yolunu yeniden yazmak. Alternatif: dosyaları olduğu gibi kopyalamak (doğrulanmamış içerik kök yazımına girerdi) |
| `serde`/`serde_json` | MIT/Apache-2.0 | ikisi | Plan, olaylar, bildirim |
| `tempfile` 3 (ağaçta; önceden yalnız dev) | MIT/Apache-2.0 | nuxut-shell `system-theme preview` | Önizleme fontlarının geçici dizini: rastgele ad, yalnız-yoksa `mkdir`, 0700, düşünce silinir (güvenlik incelemesi "insecure-temp-file", 2026-10-17). Alternatif elle rastgele ad + `create_dir`: aynı iş, daha çok kod; önceki `nuxut-system-theme-<pid>` + `create_dir_all` tahmin edilebilir ve var olanı kabul ediyordu |
| `libarchive` (`/usr/bin/bsdtar`; sistem paketi, pacman'ın bağımlılığı — Arch'ta hep kurulu) | BSD-2 | `nuxut-shell system-theme install-helper` | Kaynak kurulumunun hazırladığı kök dosyalarını tek yönetici onayıyla (`pkexec`) açar: sahibi/izni arşivde açık, mutlak yol, `..` ve ara dizin bağı varsayılan olarak reddedilir (ADR-0008 ek, 2026-10-17). Alternatif: dosya başına `pkexec install` (dosya başına ayrı parola), `pkexec sh` (kök kabuğu onayı), normal kurulumda sudo (en az ayrıcalığa aykırı) |

Çalışma zamanı (yalnız ilgili hedef için; yoksa hedef `absent`, kurulmaz): `grub` (`grub-mkconfig`,
`grub-script-check`, `grub-mkfont`), `plymouth` (two-step modülü), `limine`, `mkinitcpio`,
`polkit` (`pkexec`) + bir polkit ajanı (nuxut-polkit). Fontlar: `inter-font` (yoksa
`noto-fonts`/`ttf-dejavu`/…); eş aralıklı `ttf-jetbrains-mono` ya da `ttf-jetbrains-mono-nerd` (yoksa
Noto/DejaVu/Liberation Mono). Test: `grub-mkfont` (e2e gerçek PF2 üretir; yoksa test atlanır),
`xmllint` (polkit dosyası doğrulaması, elle).

## Geliştirme / test

| Crate / paket | Lisans | Gerekçe |
|---|---|---|
| `tempfile` | MIT/Apache-2.0 | Sahte Hyprland soketleri ve özel dbus-daemon için geçici dizin |
| `atspi` (0.29, yalnız proxy'ler), `async-io` | MIT/Apache-2.0 | `examples/at_probe`: düşmanca AT istemcisiyle güvenlik testi (2026-10-17). İkisi de `accesskit_unix` üzerinden ağaçta zaten var; ürüne girmez. Alternatif elle `zbus` çağrıları — aynı sürüm proxy'ler ek kod istemez |
| `nuxut-net` (`test-support`) | MIT | Ağ kapağı başarı/ret testleri: TLS sahte sunucu, test CA'ları, loopback çözümleyici (yalnızca testlerde; üretim istemcisi varsayılan politikada kalır, testle korunur) |
| `dbus` (`dbus-daemon`) | — | D-Bus testleri her biri kendi özel bus'ında (kullanıcı oturumuna dokunmaz) |
| `libnotify` (`notify-send`) | — | Bildirim sunucusu uçtan uca testi |
| `grim`, `ydotool` | — | Canlı görsel doğrulama (yalnızca geliştirme) |
| `xorg-xwayland`, `xclip` | — | VM pano köprüsü canlı testi: izole sway'e bağlı kendi rootless Xwayland'imiz ve kendi `xclip` süreçlerimiz (kullanıcının panosuna dokunmaz) |
| `pipewire` (`pipewire`, `pw-dump`, `pw-cli`, `pw-metadata`) | — | Ses testi kendi özel PipeWire daemon'unda (ayrı çalışma dizini ve yapılandırma, null-sink düğümleri); araçlar yalnızca o daemon'a karşı çalışır |
| `clang`, `pipewire` başlıkları (derleme) | — | `pipewire-sys`/`libspa-sys` bindgen ile derlenir (pkg-config `libpipewire-0.3`) |

## Kilit ekranı (onay 2026-10-04)

Ayrıntı ve gerekçe: `docs/ARCHITECTURE.md` "Kilit ekranı". Yalnızca `crates/nuxut-pam` ve
`lock/` (`nuxut-lock` ikilisi) kullanır; shell ikilisinin bağımlılıkları değişmez.

| Crate / paket | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `pam` (Arch, core; `libpam.so`) | GPL-2.0/BSD-3 (Linux-PAM; dinamik bağlama) | Kimlik doğrulama; her Arch kurulumunda var | — |
| `libc` 0.2 (ağaçta zaten var) | MIT/Apache-2.0 | `nuxut-pam` FFI: `malloc`/`calloc`/`free` (PAM yanıt sahipliği), `getpwuid_r`, `mlock`, `explicit_bzero` | `pam-sys` (bindgen → derlemede libclang) |
| `zeroize` 1.9 (`alloc`) | MIT/Apache-2.0 | Şifre tamponunun derleyicinin silemeyeceği şekilde sıfırlanması; RustCrypto, saf Rust, bağımlılıksız | Elle `write_volatile`: hataya açık |
| `libc::prctl` (`nuxut-pam::process`) | — | `PR_SET_DUMPABLE=0`; `unsafe` zaten yalnızca `nuxut-pam`'de olduğundan ek crate özelliği gerekmez | `rustix` `process` özelliği: ek özellik, kazanç yok |
| `tracing-subscriber`, `chrono`, `toml` (ağaçta) | MIT/Apache-2.0 | `nuxut-lock` ikilisinin logu, saat/tarih, `lock.toml` | — |
| `zbus` (ağaçta) | MIT | logind `SetLockedHint`, fprintd | — |
| `nuxut-image-sandbox` (path, framework; ağaçta, shell ile aynı) (2026-10-13) | MIT | Duvar kağıdı ve `~/.face` avatarı `nuxut-image-worker`'da duvar kağıdı profiliyle çözülür: PAM sırrını tutan kilit süreci kullanıcı dosyasını ayrıştırmaz. Çalışma zamanı: shell ile aynı işçi ikilisi (`~/.local/lib/nuxut/`), sistem paketi yok | Süreç içi çözüm (eskiden avatar: çözücü açığı kilit sürecinde çalışırdı) |
| PAM servis dosyası `/etc/pam.d/nuxut-lock` (paket kurar; içerik `auth include login`) | — | Yoksa `login` servisine düşülür (uyarıyla) | — |
| `fprintd` (opsiyonel) | GPL-2.0 (ayrı süreç, D-Bus) | Parmak izi; yoksa özellik sessizce kapalı | `pam_fprintd` ayrı PAM servisiyle (ek /etc dosyası ister) |
| `sway` (yalnızca test) | MIT | Yalıtılmış headless test compositor'ı (`nuxut-wayland` aracı) | — |

Değerlendirilip reddedilen PAM crate'leri: `pam-client` 0.5 (MPL-2.0, bindgen'li alpha
`pam-sys`), `pam` 0.8 (bakımı zayıf, şifre `CString`'de sıfırlanmıyor), `nonstick` 0.1 (genç).

## polkit ajanı (`nuxut-polkit`) (onay 2026-10-12)

Ayrıntı: `docs/design/polkit.md`, `docs/ARCHITECTURE.md` "polkit ajanı". Yeni crate yok; hepsi
ağaçta. Shell ikilisinin bağımlılıkları değişmez.

| Crate / paket | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `polkit` (Arch, extra; polkitd + `polkit-agent-helper-1`, ≥ 127 için `polkit-agent-helper.socket`) | LGPL-2.0+ (ayrı süreç, D-Bus/soket; bağlama yok) | Yetki kararı ve PAM'i root olarak çalıştıran yardımcı. Eski polkit'te setuid yardımcıya düşülür | `libpolkit-agent-1` (GObject/C bağlama, `unsafe`); protokol küçük ve belgeli |
| `nuxut-pam` (yol) | MIT | Parola tamponu (`SecretBuf`: mlock + sıfırlama), `PR_SET_DUMPABLE`, `getpwuid_r` (uid → ad, NSS uyumlu) | `uzers`: ek crate; `/etc/passwd` okuma: NSS/LDAP kullanıcılarını kaçırır |
| `rustix` 1.1 (ağaçta; `process` özelliği) | Apache-2.0/MIT | `pidfd_open`/`pidfd_send_signal` (pid yeniden kullanımına karşı güvenli sinyal), `kill`, `PR_SET_PDEATHSIG`, `getuid` — `unsafe_code = "forbid"` korunur | `libc` + `unsafe` (yalnızca `nuxut-pam`'e ait kalmalı) |
| `zbus`, `async-io`, `async-channel`, `futures-lite` (ağaçta) | MIT/Apache-2.0 | Sistem bus'ında ajan arayüzü, polkitd kaydı, logind oturumu | — |
| `serde`/`serde_json`, `toml`, `tracing(-subscriber)` (ağaçta) | MIT/Apache-2.0 | Ajan ↔ diyalog JSON sözleşmesi, `shell.toml [polkit]`, log (stderr) | — |

## Giriş ekranı (`nuxut-greeter`) (karar 2026-10-16, ADR-0010)

Yeni crate yok: hepsi ağaçta (kilitle aynı sürümler). Yeni sistem paketi yalnızca çalışma zamanı.

| Crate / paket | Lisans | Gerekçe | Alternatif |
|---|---|---|---|
| `greetd` (Arch, extra; `greeter` kullanıcısı ve `/etc/pam.d/greetd` paketle gelir) | GPL-3.0 (ayrı süreç, Unix soketi; bağlama yok) | Giriş servisi: PAM'i root olarak yapar, greeter ayrıcalıksız | SDDM, GDM, LightDM, ly (ADR-0010 karşılaştırması) |
| `hyprland` (zaten zorunlu) | BSD-3 | Greeter'ın compositor'ı (katman yüzeyleri, oturumla aynı GPU yolu) | `cage` (katman yüzeyi yok), `sway` |
| `nuxut-lock` (yol) | MIT | Ortak görünüm, durum satırları, gecikme, `LockConfig`, yalıtılmış görseller | Ayrı kopya (kural: tek yer) |
| `nuxut-pam` (yol) | MIT | `SecretBuf` (mlock + sıfırlama), `PR_SET_DUMPABLE` — PAM'i greetd yapar | — |
| `serde`/`serde_json`, `toml`, `zbus`, `zeroize`, `chrono`, `tracing` (ağaçta) | MIT/Apache-2.0 | greetd JSON'u, son seçim, logind güç eylemleri | `greetd_ipc` crate'i: ek bağımlılık; şifre yanıtını sıfırlanmayan `String`'de tutar |
| framework `nuxut` `a11y` özelliği (ağaçta: `accesskit_unix`, `atspi`; Ayarlar zaten kullanıyor) | MIT/Apache-2.0 | Giriş ekranında ekran okuyucu (AT-SPI ağacı) | Erişilebilirliksiz giriş ekranı |
| `orca` (Arch, extra; isteğe bağlı, çalışma zamanı) | LGPL-2.1 (ayrı süreç) | Giriş ekranında ekran okuyucu (`greeter.toml screen_reader = true`) | — |
| `polkit` (zaten zorunlu; `pkexec`) | LGPL-2.0 | Hoş geldin adımında kök yardımcının `greeter` alanı kullanıcı onayıyla çalışır (ADR-0008) | `sudo` (grafik onay yok) |

## Sürüm sabitleri (geçici; takip edilir)

Kalıcı değildir: her sabitin kaldırılma koşulu yazılıdır. `cargo update` sabiti korur (doğrudan
bağımlılık `=` ile); kaldırırken bu bölüm ve `Cargo.toml` satırı birlikte silinir.

### `blocking = "=1.6.2"` (2026-10-16, performans turu; takipli)

- **Ne:** zbus'ın (async-io yürütücüsü) `unblock` iş parçacığı havuzu. Kodda doğrudan
  kullanılmaz, yalnızca sürüm sabiti olarak `Cargo.toml`'da durur. Çalışma alanının tek kilidi
  olduğundan kilit ekranı, polkit ve portal da 1.6.2 alır.
- **Kök neden:** 1.7.0'a giren smol-rs/blocking PR #75 (commit `6f41360`, "Support setting the
  number of threads without an env var") havuz thread'inin çıkış koşulunu şuna çevirdi:
  `thread_count > thread_limit && (timed_out && queue.is_empty())`. Yorumlar "çok thread varsa
  ya da bir süre iş yoksa çık" der, kod ikisini birden ister. Varsayılan sınır 500 olduğundan
  koşul pratikte hiç doğru olmaz: boştaki thread çıkmaz, `wait_timeout(500 ms)` ile süreç ömrü
  boyunca saniyede 2 kez uyanır. 1.6.2'de koşul yalnızca `timed_out && queue.is_empty()`:
  500 ms boşta kalan thread çıkar. Upstream: smol-rs/blocking issue #81 "Erroneous condition when
  stopping threads" (2026-10-13 açıldı, açık, düzeltme PR'ı yok). Son sürüm hâlâ 1.7.0
  (`cargo info blocking`, 2026-10-16).
- **Ölçüm:** shell, sysmon'suz bar, 30 sn boşta: 1.7.0'da 4,1 uyanma/sn (`blocking-1/2` thread'leri
  2,0/sn her biri), 1.6.2'de 0,1/sn (`docs/research/perf-2026-10.md`).
- **Ayarla önlenebilir mi:** Hayır. 1.7.0'ın tek ayarı thread üst sınırı (`BLOCKING_MAX_THREADS`
  ya da `blocking::set_max_blocking_threads`). Thread yalnızca sayı sınırı aştığında çıkar. Sınır
  1 yapılsa bile son thread çıkmaz ve yine 2/sn uyanır. Üstelik zbus'ın bütün `unblock` işleri
  (soket bağlantısı, kimlik doğrulama dosyaları) tek thread'e sıralanır. Sabitlemek daha az zararlı.
- **Kaldırma koşulu:** issue #81'i düzelten bir `blocking` sürümü (≥ 1.7.1) çıkınca sabit
  kaldırılır. `tools/perf` ile boşta ölçüm tekrarlanır: `blocking-*` thread'leri boşta görünmemeli.
  zbus `blocking` ≥ 1.7'yi zorunlu kılarsa ve düzeltme hâlâ yoksa sabit tutulamaz. O durumda bu
  bölüme yeniden bakılır (seçenek: yamalı kopya için `[patch.crates-io]`).
- **Diğer kilitler (2026-10-16), takip:**
  - framework: aynı sabit `nuxut-portal` ve `nuxut-wayland/a11y`'de (dalı, derleme
    duraklaması bitince test edilip inecek). zbus'ı bu crate'lerden alan uygulamalar sabiti
    kendiliğinden alır.
  - settings: `nuxut/portal` üzerinden framework sabitini alır. Kilidi 1.7.0'da, bir sonraki
    çözümlemede 1.6.2'ye iner. Boşta ölçüm (1.7.0, 60 sn): 0,0 uyanma/sn; havuz thread'i
    boşta açılmamıştı. Kilit güncellemesi settings sahibine bildirildi.
  - calendar (`nuxut-calendard` sürekli çalışan daemon, zbus): kilidi 1.7.0. Framework'e
    bağlı değil; kendi sabiti gerekir. Takvim sahibine iletildi (ölçülmedi: kullanıcının çalışan
    daemon'ı ölçülmez).

## cargo-deny

`deny.toml` framework'ünkiyle birebir aynı politika (2026-10-15, sıkılaştırma 2026-10-16): lisans izin listesi (kendi crate'lerimiz dahil), `unmaintained = "all"` (dolaylı bakımsız crate de hata), kayıt bağımlılığında `*` sürüm yasak (`wildcards = "deny"`; yalnız `publish = false` path bağımlılıkları muaf), yanked/bilinmeyen kaynak reddi, OpenSSL yasağı. `ignore` listesinde tek istisna, kimlik ve gerekçeyle. Çalıştır: `cargo deny check`.

- **rustls ≥ 0.23.46** (nuxut-net / ureq üzerinden; RUSTSEC-2026-0346..0349): alt sınır framework'ün nuxut-net manifestinde.
- **ttf-parser 0.25 bakımsız** (RUSTSEC-2026-0192, düzeltilmiş sürüm yok): dolaylı (framework nuxut-render → cosmic-text 0.19 → fontdb 0.23); deny.toml `ignore` listesinde açıkça istisna. Güvenlik açığı değil, bakım uyarısı. fontdb 0.24 ttf-parser'ı bırakıyor; cosmic-text ona geçince ağaçtan çıkar — framework takibi (karar 2026-10-15).
