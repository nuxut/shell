# Nuxut — Temel Araştırma Raporu

> Tarih: 2026-10-04 · Kapsam: referans repolar, `old/` projesi, UI/UX, Hyprland ortam gereksinimleri, framework seçenekleri
> Bu belge karar vermek için hazırlandı; **karar vermez**. Kararlar `docs/decisions/` altında ADR olarak tutulmalı.

---

## 0. Özet

1. **Üç popüler shell'in ortak sırrı framework değil, tasarım sistemi.** Üçü de token tabanlı (renk, radius, süre, easing) tek kaynaktan beslenen, Material 3 türevi bir görsel dil kullanıyor, duvar kağıdından dinamik palet üretiyor ve bu paleti **shell dışındaki uygulamalara da** (GTK, Qt, terminal, editör) yayıyor. İnsanlar "shell"i değil, **bütünüyle tutarlı bir masaüstünü** seviyor.
2. **Quickshell (Qt/QML) fiilî standart haline geldi**: end-4 AGS'den, caelestia sıfırdan, noctalia v1–v4 ona geçti. Ama **noctalia v5 Quickshell'i terk etti** ve doğrudan Wayland + OpenGL ES üzerinde C++ ile her şeyi yeniden yazdı (bildirilen sebep: monitör başına yüzlerce MB bellek; v5 ile ~6× azalma). Bu, "kendimiz yazalım" seçeneğinin hem *mümkün* hem *çok pahalı* olduğunun canlı kanıtı: **~323 bin satır C++, 6 ay, çok sayıda katkıcı.**
3. **"Shell + cross-platform uygulamalar" hedefi framework seçimini kökten değiştiriyor.** Sadece Linux shell olsaydı noctalia yolu cazip olurdu. Ama aynı tasarım dilini Windows/macOS/(mobil) uygulamalara taşımak istiyorsan, kendi renderer'ını yazmak = **kendi GUI toolkit'ini yazmak** (metin girişi, IME, erişilebilirlik, pencereleme, DnD, clipboard…). Bu, yıllarca süren bir iştir ve shell'in kendisinden büyüktür.
4. **Gerçekçi adaylar:** (A) **Qt 6 / QML** — Quickshell ile shell + Qt Quick ile uygulamalar, tek QML bileşen kütüphanesi; (B) **Rust + Slint** — hafif, gerçek cross-platform, ama layer-shell desteği üçüncü parti ve olgunlaşmamış; (C) **Rust + iced/libcosmic** — COSMIC kanıtlıyor, ama estetik/animasyon esnekliği daha sınırlı. GTK4 (eski proje) cross-platform hedefi için zayıf.
5. **Öneri:** Karardan önce, aynı küçük dilimi (bar + açılır panel + bildirim + animasyon + çoklu monitör) **iki stack'te** (A ve B) yazıp ölçmek. Bellek/CPU/kare süresi + geliştirme hızı + "göze nasıl göründüğü". Bir haftalık spike, yıllık bir hatadan ucuzdur.
6. **`old/` projesinden en büyük ders teknik değil, süreç dersi:** 12 rollü süreç bürokrasisi ~1 haftada ~3.600 satır (bar + saat + workspace) üretti; hafızaların çoğu ürün hakkında değil, sürecin kendi sorunları hakkında. Teknik notlar (Hyprland IPC gotcha'ları, config feedback loop, matugen'in CLI olması) ise değerli ve korunmalı.

---

## 1. Referans repolar

### 1.1 Sayılar

| | caelestia-shell | end-4/dots-hyprland (illogical-impulse) | noctalia |
|---|---|---|---|
| Yıldız (2026-10) | ~12.7k | ~16.3k | ~11.1k |
| Başlangıç | 2025-01 | 2023-12 (AGS), 2025-04'te Quickshell'e geçiş | v5 reposu 2026-04 (v1–v4 Quickshell) |
| Commit | ~2.7k | ~6.3k | ~5.7k (yalnız v5) |
| Teknoloji | Quickshell (QML) + C++ Qt plugin | Quickshell (QML) + kurulum betikleri | Saf C++23, Wayland + EGL/GLES, Pango/Cairo metin |
| Boyut | ~40k satır QML + ~20k satır C++ | ~57k satır QML (586 dosya) + tam dotfiles | ~323k satır C++ |
| Config | `~/.config/caelestia/shell.json` + monitör başına override | Quickshell içi ayar uygulaması + JSON | TOML + hot reload + GUI override katmanı |
| Kapsam | Sadece shell (dotfiles ayrı repo) | Tüm masaüstü: Hyprland, fish, kitty, Kvantum, KDE renkleri, fontlar, installer (Arch/Fedora/Gentoo/Nix) | Shell + plugin sistemi (Luau), greeter ayrı proje |
| Compositor | Hyprland | Hyprland | Niri, Hyprland, Sway, Labwc, dwl, Mango… |

### 1.2 Neden popülerler?

**Ortak nedenler (asıl ders bunlar):**
- **Video'da güzel görünüyorlar.** Hareket (motion) birinci sınıf vatandaş. README'nin ilk öğesi bir video. r/unixporn kültürü görsel; animasyonlu bir demo statik ekran görüntüsünü her zaman yener.
- **Tek bütün hissi.** Bar, launcher, bildirim, kilit ekranı, OSD, medya, dashboard aynı dilden konuşuyor. Kullanıcı 10 farklı aracı (waybar + rofi + dunst + swaylock + …) birbirine yapıştırmak zorunda kalmıyor.
- **Dinamik renk.** Duvar kağıdını değiştirince *her şey* (shell + GTK + Qt + terminal) yeni paleti alıyor. Bu, "vay" etkisinin büyük kısmı.
- **Kur-ve-çalıştır.** AUR paketi, Nix flake, installer. Rice'ı kopyalamanın acısını alıyorlar.
- **Aktif bakım + topluluk** (Discord, sık commit, hızlı bug fix).

**Her birinin farkı:**

**caelestia — "zanaatkâr estetiği"**
- En "imza" görünüme sahip olan: ekranın etrafında ince bir **çerçeve (frame)** ve panellerin bu çerçeveden **organik olarak büyüyerek** çıkması (`modules/drawers/` — tüm paneller tek bir tam-ekran yüzeyde, şekiller birbirine akıyor).
- C++ plugin'de özel **"Blobs"** shader'ları (morph eden şekiller), `WavyLine`, `VisualiserBars`, `SparklineItem`, `LazyListView` gibi performans için native'e indirilmiş bileşenler. Yani: *QML ile iterasyon hızı, darboğazda C++.*
- **Material 3 Expressive hareket token'ları** (`components/Anim.qml`: `StandardSmall…EmphasizedExtraLarge`, `FastSpatial/DefaultSpatial/SlowSpatial`, `*Effects`). Her animasyon bir *tür* seçer, süre/easing token'dan gelir → tutarlılık.
- `StyledRect` gibi temel bileşenler renk değişimini otomatik animasyonlu yapar → "her şey yumuşak geçiyor" hissi bedavaya gelir.
- Zayıf yönü: tek kişilik vizyon; Hyprland'a sıkı bağlı; config manuel JSON.

**end-4 (illogical-impulse) — "her şey dahil"**
- En kapsamlısı: sohbet asistanı kenar paneli, çeviri, booru, OCR, ekran kaydı, şarkı tanıma (songrec), LaTeX, todo, cliphist, emoji, hyprsunset, anti-flashbang shader…
- **İki "panel ailesi"**: `ii` (Material) ve `waffle` (Windows 11 benzeri). Aynı servis katmanı üzerinde iki farklı görsel dil → **servis/görünüm ayrımının** iyi bir örneği.
- **Tüm sistemi temalıyor**: matugen şablonları ile GTK3/4, Kvantum/Darkly (Qt), KDE renkleri, fuzzel, kitty, foot… Dağıtım bazında installer + bağımlılık meta-paketleri (`sdata/deps-info.md` — Hyprland ortamında ne gerektiğinin en iyi listesi).
- Zayıf yönü: dev bir bağımlılık yüzeyi (KDE parçaları, Python venv, pinned quickshell-git); "tak-çıkar" değil, "ya hep ya hiç".

**noctalia — "mühendislik ve verimlilik"**
- v5 ile Qt/GTK yok: kendi event loop'u (poll tabanlı), kendi scene graph'ı, kendi GLES renderer'ı, Pango/Cairo ile metin.
- **`ARCHITECTURE.md` okunmalı** — mükemmel bir disiplin örneği: kare fazları (`PrepareFrame → Update → Layout → Render`, Render'da asla layout yok), katman sınırları (shell kodu ham node yaratamaz, sadece kontroller), fractional scaling kuralları, metin dikey hizalama (cap-height'a göre ortalama, erken yuvarlama yok) gibi **piksel-mükemmellik invariant'ları**.
- **Multi-compositor** (ext-workspace-v1 + compositor'e özel backend'ler), **plugin API** (Luau), TOML + hot reload, GUI'den yapılan değişikliklerin ayrı override katmanında tutulması (kullanıcının elle yazdığı config'i bozmaz).
- Material Color Utilities vendored → matugen'e dış bağımlılık yok; uygulama temalama şablonları yerleşik.
- Zayıf yönü: dev kod tabanı, sadece Linux, her yeni görsel efekt = shader + backend kodu.

### 1.3 Çıkarılacak dersler (tasarım kuralları olarak)

1. **Tasarım token'ları tek kaynak** (renk, tipografi ölçeği, spacing, radius, elevation, **motion**). Bileşenler token tüketir, asla sabit değer içermez.
2. **Motion bir sistemdir, süs değil.** Animasyon *türleri* tanımla (spatial/effects × fast/default/slow), her geçiş bir tür seçsin. Kesintiye uğrayabilen (interruptible) animasyonlar.
3. **Servis katmanı ≠ görünüm katmanı.** (end-4'ün iki panel ailesi.) Servisler UI'dan bağımsız test edilebilir olmalı ve uygulamalar tarafından da kullanılabilmeli.
4. **Temel bileşenler varsayılan olarak doğru davranır** (renk geçişi animasyonlu, hover/press state layer'ı, focus ring). Tutarlılık disiplinle değil, *varsayılanla* sağlanır.
5. **Piksel invariant'ları belgelenir** (noctalia): fractional scaling, metin hizalama, yuvarlama kuralları.
6. **Config katmanlıdır:** varsayılan → kullanıcı dosyası → monitör başına override → GUI override. Hot reload. Bilinmeyen anahtar hatası kullanıcıya gösterilir.
7. **Sistem çapında temalama** shell'in parçasıdır (GTK/Qt/terminal şablonları).
8. **Performans kritik yerler native**, geri kalanı hızlı iterasyon dilinde (caelestia modeli).
9. **Kapsamı yaz** (noctalia "Scope" bölümü): shell neyi yapar, neyi compositor'e/uygulamalara bırakır.

---

## 2. `old/` projesi (nuxut-shell, Rust + GTK4 + Relm4)

**Ne yapılmış:** iced → GTK4 spike geçişi (2026-06-14), Relm4 bileşen mimarisi, gtk4-layer-shell, tam ekran **çerçeve + bar** (Cairo ile even-odd çizim, input/opaque region yönetimi, kenar başına alan rezervasyonu), TOML config + self-heal, tracing log, i18n (TR/EN, stack-key), ham soket ile Hyprland IPC (workspace + odaklı pencere). ~3.600 satır Rust, 11 commit, ~1 hafta.

**Korunacak teknik bilgi (gerçekten değerli):**
- Hyprland IPC gotcha'ları: socket1 EOF modeli, `read_timeout` zorunluluğu, `activeWorkspace` camelCase istisnası, event-burst debounce, `$XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE`, compositor restart'ta reconnect.
- **Config save-on-parse + FileMonitor = sonsuz döngü** (saniyede ~2270 reload). Her hot-reload sisteminde tekrar karşımıza çıkacak.
- Bloklayan okuma yapan thread'i `AtomicBool` ile durduramazsın; soketi kapatmak gerekir.
- matugen bir kütüphane değil CLI'dır → ya alt süreç ya şablon çıktısı (ya da Material Color Utilities'i doğrudan kullan, noctalia gibi).
- GTK `window`'a otomatik opak `.background` basar; şeffaflık için bare-element selector gerekir.
- Qt uygulamaları tema değişikliği için yeniden başlatma ister, GTK canlı yükler.
- Görsel testler sandbox'ta yapılamaz → **ekran görüntüsü tabanlı doğrulama döngüsü** şart.

**Dürüst değerlendirme:**
- Çerçeve fikri caelestia ile aynı yönde ve iyi bir imza; ama GTK4 + Cairo ile organik "panelin çerçeveden büyümesi" animasyonu zor (GTK'da shader/scene graph kontrolü sınırlı).
- **Süreç ağır bastı.** 12 rol (PM, architect, backlog, compliance, reviewer, tester, librarian…), sprint/WG/§ numaraları, raporlar. Hafıza dosyalarının çoğu ürünü değil süreci anlatıyor ("worker raporu diske yazmıyor", "stray Departments tree", "log annotation grep tuzağı"). Bu kadar seremoni tek kişilik bir projede **hızı öldürür ve dikkati yanlış yere çeker.**
- Yeni projede önerim: hafif süreç; belgeler `docs/` içinde (ADR + tasarım sistemi + mimari invariant'lar); doğrulama = build + test + ekran görüntüsü.

---

## 3. UI/UX: iyi tasarım nasıl olur?

### 3.1 Masaüstü shell'i için UI/UX ilkeleri

- **Shell görünmez olmalı, ta ki ihtiyaç duyulana kadar.** İçerik (uygulamalar) kahramandır; shell sakin, bilgi yoğunluğu ölçülü.
- **Hız bir estetiktir.** Açılış < 100 ms algısı, girişe anında tepki (input latency), takılmayan 60/120/144 Hz. Animasyon hiçbir zaman girdiyi bloklamamalı ve kesilebilmeli.
- **Mekânsal tutarlılık:** bir panel nereden çıkıyorsa oraya geri dönmeli (caelestia çerçevesi bunun iyi örneği). Hareket, nesnenin nereden geldiğini anlatmalı.
- **Klavye birinci sınıf** (Hyprland kullanıcısı klavyeyle yaşar): her şey klavyeyle ulaşılabilir, odak görünür, Esc her zaman kapatır.
- **Tipografi ölçeği + 4/8 px spacing ızgarası + sınırlı radius seti.** Az sayıda değer, sıkı uygulanmış.
- **Kontrast ve erişilebilirlik:** WCAG kontrast oranları dinamik palette de korunmalı (Material'ın tonal palet sistemi bunu garanti etmeye yardımcı).
- **Durum tasarımı:** boş/yükleniyor/hata/çevrimdışı durumları baştan tasarlanır (bluetooth kapalı, ağ yok, pil yok…).
- **Kişiselleştirme ≠ sınırsız seçenek.** İyi varsayılanlar + birkaç güçlü düğme (renk kaynağı, yoğunluk, motion yoğunluğu, köşe yuvarlaklığı, şeffaflık). Gerisi config/plugin.


---

## 4. Hyprland ortamında neler gerekiyor?

Bir shell'in ya **kendisi sağladığı** ya da **konuştuğu** servisler. (Geliştirme ortamında şu an çalışanlar: Hyprland 0.56.2, PipeWire + WirePlumber, NetworkManager, UPower, polkitd + hyprpolkitagent, gnome-keyring, xdg-desktop-portal(-hyprland/-gtk/-kde), dunst kurulu, waybar/quickshell kurulu. `bluetoothd` çalışmıyor görünüyor. Performans ölçümleri gerçek donanımda yapılmalı.)

### 4.1 Shell'in *sağlaması* gerekenler (sunucu tarafı rolü)

| Rol | Protokol | Not |
|---|---|---|
| Bildirim sunucusu | D-Bus `org.freedesktop.Notifications` | dunst/mako/swaync'nin yerini alır. Aksiyonlar, resimler, gruplama, geçmiş, DND. |
| Sistem tepsisi | `org.kde.StatusNotifierWatcher/Item` + `com.canonical.dbusmenu` | Discord, Steam, Nextcloud vb. için şart. |
| Polkit ajanı | `polkit-agent-1` | hyprpolkitagent'ın yerine kendi şık diyaloğumuz (opsiyonel, güvenlik kritik). |
| Kilit ekranı | `ext-session-lock-v1` + PAM (+ fprintd) | hyprlock alternatifi. **Güvenlik kritik**: çökerse oturum kilitli kalmalı. |
| Idle | `ext-idle-notify-v1`, `idle-inhibit` | hypridle alternatifi veya onunla birlikte. |
| Launcher | `.desktop` dosyaları, `xdg-activation-v1` | fuzzy arama, hesap makinesi (libqalculate), komutlar. |
| Pano geçmişi | `ext-data-control-v1` / `wlr-data-control` | cliphist alternatifi. |
| OSD | ses/parlaklık/mikrofon/caps lock | |
| Duvar kağıdı | layer-shell background katmanı | hyprpaper/swww yerine. |
| Medya kontrolü | MPRIS (D-Bus) | playerctl'in yaptığı. |
| Ekran görüntüsü/kayıt UI | grim/slurp/wf-recorder veya portal | |

### 4.2 Shell'in *istemcisi* olduğu servisler

| Alan | Servis / API | Not |
|---|---|---|
| Compositor | Hyprland IPC (socket1 komut, socket2 olay) + Wayland protokolleri (`foreign-toplevel`, `ext-workspace-v1`) | Multi-compositor istiyorsak protokol öncelikli, IPC ek. **Hyprland artık Lua config de destekliyor** (`hyprland.lua`); sen hâlâ `.conf` kullanıyorsun. |
| Ses | PipeWire + WirePlumber (`libpipewire`, `wireplumber`) | Cihaz/akış başına ses, varsayılan çıkış, mikrofon. |
| Ağ | NetworkManager D-Bus | Wi-Fi tarama/bağlanma, VPN, hotspot. Gizli ağ şifreleri için secret agent. |
| Bluetooth | BlueZ D-Bus (`org.bluez`) + eşleştirme için Agent | `bluetoothd` servisi etkin olmalı. Pil seviyesi (`Battery1`). |
| Güç | UPower (pil), power-profiles-daemon veya tuned-ppd (profil), logind (uyku/kapatma, inhibit) | `powerprofilesctl` kurulu değil. |
| Parlaklık | backlight (logind `SetBrightness` veya brightnessctl), harici monitör: ddcutil (DDC/CI) | |
| Gece ışığı | hyprsunset / `wlr-gamma-control` | |
| Medya | MPRIS | |
| Kimlik | gnome-keyring / KWallet (Secret Service) | |
| Portallar | xdg-desktop-portal + `-hyprland` (ekran paylaşımı) + `-gtk` (dosya seçici) | Portal sırası `hyprland-portals.conf` ile. |
| Takvim/hava | CalDAV/Google, Open-Meteo vb. | noctalia hepsini içeriyor. |
| Sistem bilgisi | `/proc`, `/sys`, hwmon, GPU (nvidia-smi/amdgpu sysfs) | |
| Tema yayılımı | GTK (gsettings + `gtk.css`), Qt (qt6ct/Kvantum/Darkly), kitty/foot, vb. | matugen veya gömülü Material Color Utilities. |

### 4.3 Tavsiye edilen temel paket kümesi (Arch)

`hyprland xdg-desktop-portal-hyprland xdg-desktop-portal-gtk pipewire wireplumber pipewire-pulse networkmanager bluez bluez-utils upower power-profiles-daemon brightnessctl ddcutil polkit gnome-keyring grim slurp wl-clipboard cliphist hypridle hyprlock hyprpicker hyprsunset playerctl libqalculate matugen` + fontlar (bir sans, bir mono Nerd Font, Material Symbols). end-4'ün `sdata/deps-info.md` dosyası paket paket gerekçe veriyor.

---

## 5. Framework seçenekleri

Değerlendirme ölçütleri: **(E)** estetik/animasyon tavanı · **(P)** performans/bellek · **(K)** kişiselleştirme (hot reload, kullanıcı tarafından değiştirilebilirlik) · **(X)** cross-platform uygulamalar · **(T)** tek tasarım dili ile shell+uygulama tutarlılığı · **(R)** risk/olgunluk · **(H)** iterasyon hızı.

### A) Qt 6 / QML — Quickshell (shell) + Qt Quick (uygulamalar)
- **E: çok yüksek.** Scene graph, shader efektleri, `Behavior`/state geçişleri, MultiEffect (blur/gölge). caelestia/end-4 tavanı gösteriyor.
- **P: orta.** Qt + V4 JS motoru; monitör başına bellek maliyeti gerçek (noctalia'nın ayrılma sebebi). İyi yazılmış QML (lazy `Loader`, native plugin ile ağır işler, `QS_DROP_EXPENSIVE_FONTS`) bunu azaltır ama sıfırlamaz. **Ölçmeden karar verilmemeli.**
- **K: çok yüksek.** QML dosya izleme ile canlı yeniden yükleme; kullanıcı QML ile genişletebilir.
- **X: çok yüksek.** Qt Quick Linux/Windows/macOS/Android/iOS'ta gerçekten çalışır. Erişilebilirlik, IME, metin, HiDPI olgun.
- **T: çok yüksek.** **Aynı QML bileşen kütüphanesi** hem shell'de hem uygulamalarda. Bu, "tutarlılık" hedefi için en güçlü argüman.
- **R:** Quickshell tek ana geliştiriciye bağlı ve git sürümü gerektiriyor; Qt lisansı LGPL (uygun).
- **H: çok yüksek.**

### B) Rust + Slint (shell: layer-shika veya kendi entegrasyonumuz; uygulamalar: Slint)
- **E: yüksek ama tavanı A'dan düşük.** Deklaratif `.slint` dili, animasyonlar, state'ler var; özel shader/efekt yazmak sınırlı.
- **P: çok yüksek.** Küçük bellek ayak izi (gömülü sistemler için tasarlanmış), JS motoru yok.
- **K: orta-yüksek.** Live preview var; kullanıcı eklentisi için yorumlayıcı modu (`slint-interpreter`) mümkün.
- **X: yüksek.** Linux/Windows/macOS/web, Android; iOS daha yeni.
- **T: yüksek.** Tek dil (Rust) + tek UI dili.
- **R: yüksek risk shell tarafında.** Slint'in yerleşik layer-shell desteği yok; `layer-shika` 1.0 öncesi, ~50 yıldız. Büyük ihtimalle bu katmanı **kendimiz yazmamız/fork'lamamız** gerekir. Lisans: royalty-free (atıf şartlı) / GPLv3 / ticari.
- **H: yüksek** (Rust derleme süreleri iterasyonu yavaşlatır; interpreter ile hafifler).

### C) Rust + iced / libcosmic (shell: `iced_layershell` veya cosmic'in sctk entegrasyonu)
- **E: orta.** Elm mimarisi temiz; ama karmaşık animasyon/efekt tarihsel olarak zayıf, görünüm "uygulama gibi" kalma eğiliminde.
- **P: yüksek.** **X: yüksek.** **T: yüksek.**
- **R: orta.** COSMIC (1.2 stabil, 2026-07) koca bir masaüstünü bunun üzerinde kanıtladı; libcosmic'in görsel dili COSMIC'e bağlı.
- `old/` ekibi iced'den GTK'ya geçmişti — sebebi tekrar değerlendirilmeli.

### D) GTK4 (Rust gtk4-rs/Relm4 veya AGS/Astal)
- **E: orta** (CSS alt kümesi, shader yok, Cairo). **X: zayıf** (Windows/macOS'ta GTK deneyimi kötü). **T:** Linux'ta iyi, dışarıda zayıf.
- Cross-platform uygulama hedefiyle **elenir** (eski projenin yönü buydu).

### E) Kendimiz yazmak (noctalia v5 yolu): Rust + wgpu/GLES + vello/tiny-skia + parley (metin) + accesskit + smithay-client-toolkit
- **E: sınırsız.** **P: en iyi.** **K: biz ne tasarlarsak.**
- **X/T: çok pahalı.** Shell Linux-only olduğu için yalnızca Wayland yeter; ama uygulamaları Windows/macOS'a taşımak için pencereleme (winit), metin düzenleme, IME, erişilebilirlik, kaydırma fiziği, sanal listeler, DnD, clipboard, font fallback, RTL… hepsini yazmak gerekir. Bu, Xilem/Masonry ve Slint'in yıllardır çözmeye çalıştığı problem.
- **Dürüst maliyet tahmini:** noctalia v5 ≈ 323k satır, ~6 ay, kalabalık ekip, mevcut bir v4 tecrübesi ile. Sadece shell için tek kişiyle **1–2 yıl** ciddi iş; uygulama toolkit'i eklenince **çok yıllı**.
- **Ne zaman mantıklı:** Hedef "dünyanın en verimli shell'i" ve uygulamalar ayrı bir stack ile yazılacaksa. "Tek tasarım dili ile shell + cross-platform uygulama" hedefinde, toolkit'i yeniden yazmak çekirdek değeri (tasarım + deneyim) geciktirir.

### F) Web tabanlı (Tauri/Electron/WebView) ve Flutter
- Web: uygulamalar için iyi, shell için bellek/başlangıç maliyeti yüksek; "gerçekten performanslı" hedefiyle çelişir.
- Flutter: uygulamalar için mükemmel cross-platform (mobil dahil), ama Wayland layer-shell desteği marjinal ve bellek ağır.

### 5.1 Tablo

| | E | P | K | X | T | R (düşük=iyi) | H |
|---|---|---|---|---|---|---|---|
| A Qt/QML + Quickshell | ★★★★★ | ★★★ | ★★★★★ | ★★★★★ | ★★★★★ | orta | ★★★★★ |
| B Rust + Slint | ★★★★ | ★★★★★ | ★★★★ | ★★★★ | ★★★★ | yüksek (shell) | ★★★ |
| C Rust + iced | ★★★ | ★★★★ | ★★★ | ★★★★ | ★★★★ | orta | ★★★ |
| D GTK4 | ★★★ | ★★★★ | ★★★ | ★★ | ★★ | düşük | ★★★ |
| E Kendi renderer | ★★★★★ | ★★★★★ | ★★★★★ | ★ (pahalı) | ★★★★★ (pahalı) | çok yüksek | ★ |
| F Web/Flutter | ★★★★ | ★★ | ★★★★ | ★★★★★ | ★★★★ | orta | ★★★★ |

### 5.2 Önerilen mimari (framework'ten bağımsız)

```
┌───────────────────────────────────────────────────────────┐
│  Tasarım sistemi (tek kaynak): tokens.toml                │
│  renk rolleri · tipografi · spacing · radius · motion     │
│  → üreteçler: QML/Slint · GTK css · Qt/Kvantum · kitty …  │
├───────────────────────────────────────────────────────────┤
│  Bileşen kütüphanesi (Button, Slider, List, Panel, …)     │
│  + bileşen galerisi (görsel test)                         │
├───────────────────────────┬───────────────────────────────┤
│  Shell yüzeyleri          │  Uygulamalar (cross-platform) │
│  bar, launcher, bildirim, │  ayarlar, dosya?, medya? …    │
│  kilit, OSD, dashboard    │                               │
├───────────────────────────┴───────────────────────────────┤
│  Servis katmanı (Linux): Hyprland/Wayland, PipeWire, NM,  │
│  BlueZ, UPower, MPRIS, Notifications, Tray, Polkit …      │
│  — UI'dan bağımsız, test edilebilir; native (C++/Rust)    │
└───────────────────────────────────────────────────────────┘
```

Bu katmanlama sayesinde framework kararı yanlış çıksa bile **token'lar, tasarım belgeleri, servis katmanı ve testler** korunur. Karar geri döndürülebilir hale gelir.

### 5.3 Karar önerisi

1. **Spike (1–2 hafta):** Aynı dilim → (A) Quickshell/QML ve (B) Rust+Slint. Dilim: çerçeve + bar (workspace, saat, ses), çerçeveden büyüyen bir panel animasyonu, bildirim pop-up'ı, 2 monitör, dinamik palet.
2. **Ölçüm:** RSS (monitör başına), boşta CPU, panel açılış gecikmesi, kare süresi (120 Hz'de takılma), soğuk başlangıç; + öznel: göz, kod okunabilirliği, değişiklik başına süre.
3. **ADR yaz**, karar ver.

Benim mevcut eğilimim (ölçümden önce): **"tek tasarım dili + cross-platform uygulama"** hedefi en çok A'yı destekliyor; performans hedefi B'yi. Performans sorununu A'da native plugin + disiplinle yönetmek, B'de eksik layer-shell/efekt altyapısını sıfırdan yazmaktan **muhtemelen** daha ucuz. Ama bu tam olarak ölçümle çürütülmesi gereken bir hipotez.

---

## 6. Çalışma ortamı önerisi

```
nuxut/shell/
├── README.md                 # kısa yönlendirme: amaç, klasörler, kurallar, doğrulama komutları
├── docs/
│   ├── PRODUCT.md            # kimin için, hangi his, ne değiliz
│   ├── design/DESIGN.md      # token'lar, bileşen kuralları, yasaklar
│   ├── design/moodboard/     # referans ekran görüntüleri + notlar
│   ├── architecture/         # invariant'lar (noctalia ARCHITECTURE.md tarzı)
│   ├── decisions/            # ADR-0001-framework.md …
│   └── research/             # bu rapor
├── references/               # caelestia-shell, dots-hyprland, noctalia, old (salt-okunur)
├── spikes/                   # framework karşılaştırma prototipleri
└── (ileride) src/ veya crates/
```

- **Görsel doğrulama:** nested Hyprland (`Hyprland` içinde `Hyprland`) veya ayrı TTY oturumu + `grim` ile ekran görüntüsü betiği.
- **Süreç:** kısa iterasyonlar, her karar ADR, her görsel değişiklik ekran görüntüsüyle doğrulanır.

---

## Kaynaklar

- Noctalia v5 yeniden yazımı ve bellek — https://byteiota.com/noctalia-5-wayland-desktop-shell-stable-cpp-rewrite/ , https://www.linuxcompatible.org/story/noctalia-501-stable-release-groundup-rewrite-and-6x-memory-reduction-for-linux/
- layer-shika (Slint + layer-shell) — https://github.com/waydeerwm/layer-shika
- Slint lisansı — https://slint.dev/blog/slint-1.1-released
- iced_layershell — https://lib.rs/crates/iced_layershell
- COSMIC — https://en.wikipedia.org/wiki/COSMIC_desktop
- Yerel: `noctalia/ARCHITECTURE.md`, `noctalia/CONTRIBUTING.md`, `dots-hyprland/sdata/deps-info.md`, `caelestia-shell/components/Anim.qml`
