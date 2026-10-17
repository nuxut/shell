# Masaüstü servisleri: boşluk analizi ve yol haritası

İlk analiz 2026-10-12 (görev; iki tarama turu, önemli iddialar
elle doğrulandı), bu özet 2026-10-13. Her yeteneğin ayrıntılı tasarımı kendi notunda; burada
yalnızca durum, öncelik, karar ve sahiplik.

Ortam (analiz anı): Hyprland 0.56, polkit 127 (yardımcı setuid değil,
`/run/polkit/agent-helper.socket`), xdg-desktop-portal 1.22, upower 1.91, NetworkManager 1.58,
bluez 5.87.

## Durum

Durum: **VAR** Nuxut yapıyor · **KISMEN** eksik parçası var · **YOK** · **DIŞ** kullanıcının
Hyprland zincirindeki Nuxut dışı bir araç yapıyor.

| # | Yetenek | Durum (2026-10-13) | Tasarım / kod | Eksik |
|---|---|---|---|---|
| 1 | polkit kimlik doğrulama ajanı | VAR | `docs/design/polkit.md`, `polkit/` (`nuxut-polkit`) | Kullanıcının hyprpolkitagent'ından geçişi kendisi seçer (install.sh yönergesi). |
| 2a | Portal: Settings (görünüm) | KISMEN — inişte | `docs/design/portal.md`, `portal/` (`xdg-desktop-portal-nuxut`) | uwsm oturumu doğrulanmadı (ARCHITECTURE "Portal arka ucu"). |
| 2b | Portal: FileChooser | DIŞ | Kullanıcının seçimi korunur (`nuxut-portals.conf` türetilir) | Nuxut dosya seçici (L; framework dosya listesi bileşeni gerekir). |
| 2c | Portal: Screenshot/ScreenCast | DIŞ | xdg-desktop-portal-hyprland | Nuxut paylaşım seçicisi (sonra). |
| 3 | Ekran görüntüsü / bölge / kayıt | VAR (görüntü), kayıt v1.1 | `docs/design/screenshot.md` | Kayıt (wf-recorder) + bar göstergesi. |
| 4 | Pano geçmişi | KISMEN — tasarım | `docs/design/clipboard.md` | framework data-control yeteneği, başlatıcı kipi. |
| 5 | Gece ışığı | YOK — tasarım + servis hazır | `docs/design/night-light.md` | framework CTM yeteneği, gerçek arka uç. |
| 6 | Wi-Fi / VPN | KISMEN | `src/services/network/`, hızlı panel | Ayarlar sayfası, gizli ağ, 802.1X, VPN içe aktarma; NM istemcisinin paylaşılan crate'e çıkması. |
| 7 | Bluetooth eşleştirme | KISMEN | `src/services/bluetooth/` (ajan tam), hızlı panel | Ayarlar sayfası (cihaz yönetimi, ad, görünürlük). |
| 8 | Varsayılan uygulamalar / MIME | YOK — sözleşme onaylı (defaults-v1, t116; repoya `docs/design/defaults.md` olarak girecek) | — | Uygulama (kabuk + Ayarlar). |
| 9 | Güç profilleri / pil eşiği | KISMEN | `src/services/power/` | UPower `EnableChargeThreshold`, Güç sayfası. |
| 10 | Kullanıcı adı / avatar | KISMEN (okuma) | lock, oturum kartı | accountsservice `SetIconFile`/`SetRealName`. |
| 11 | Tarih-saat / NTP / saat dilimi | YOK — karar var | t119 | Ayarlar timedated'i doğrudan çağırır, kabuk TZ'yi yeniler; polkit ajanı hazır. |
| 12 | Klavye düzeni | VAR (ayar), bar göstergesi yok | `docs/design/input.md` (input-v1) | Bar göstergesi (`activelayout`). |
| 13 | Fare / dokunmatik yüzey | VAR | `docs/design/input.md` | — |
| 14 | Yazıcılar | YOK | — | Harici araca bağlantı (system-config-printer), isteğe bağlı paket notu. |
| 15 | Çıkarılabilir medya | KISMEN — v1 (t122, inişte) | `docs/design/removable-media.md`, `src/services/removable_media/` | Hızlı panel listesi, kilitli kripto için kilit açma istemi. |
| 16 | Ekran klavyesi | YOK | — | Harici araç (wvkbd), isteğe bağlı. |
| 17 | Emoji seçici | YOK | — | Başlatıcı kipi (`emojis` crate). |
| 18a | Büyüteç | DIŞ | — | `cursor:zoom_factor` kısayolları + Ayarlar. |
| 18b | Yüksek karşıtlık | VAR | Framework `[colors] contrast` | Framework tema kipi; portal `contrast` = 1 (yüksek) / 0 (standart). |
| 18c | Metin ölçeği | VAR | Ayarlar yazı tipi sayfası | GTK/Qt'ye `text-scaling-factor` aktarımı. |
| 18d | Azaltılmış hareket | VAR | Hareket "Kapalı"; portal `reduced-motion` | GTK `gtk-enable-animations` aktarımı. |

## Öncelik (etki / maliyet)

Maliyet: S ≤ 1 iş günü, M 2–3, L 4+. Sıra onaylı (2026-10-12); 1–3 tamamlandı.

| Sıra | İş | Etki | Maliyet |
|---|---|---|---|
| 1 | polkit ajanı (9, 11, 15, 10'un önünü açar) | Yüksek | M |
| 2 | Ekran görüntüsü + kayıt | Yüksek | M + S |
| 3 | Portal arka ucu: Settings | Orta-yüksek | S–M |
| 4 | Gece ışığı | Orta | S–M |
| 5 | Klavye/fare (input-v1 ile yapıldı) | Orta-yüksek | S–M |
| 6 | Wi-Fi/VPN Ayarlar sayfası | Orta | L |
| 7 | Pano geçmişi + emoji (başlatıcı kipleri) | Orta | M + S |
| 8 | Çıkarılabilir medya bildirimi | Orta | M |
| 9 | Tarih-saat/NTP; güç sayfası + pil eşiği; avatar | Düşük-orta | her biri S |
| 10 | Bluetooth sayfası, varsayılan uygulamalar, büyüteç, yüksek karşıtlık | Düşük-orta | M |
| 11 | Yazıcılar, ekran klavyesi (harici araç) | Düşük | — |

## Kararlar

- **polkit ayrı ikili** (`nuxut-polkit`, öneri, onay 2026-10-12):
  parola yalnızca istek başına diyalog sürecinde; kabuk çökse de ajan kayıtlı kalır. libpolkit-agent
  yok, tamamı zbus. Etkinlik kullanıcının seçimi (başka polkit ajanı varsa etkinleştirilmez).
- **Ekran görüntüsü kendi seçim arayüzüyle** (slurp yerine; yakalama v1 `grim` alt süreci,
  ileride framework screencopy).
- **Portal seçimi `XDG_CURRENT_DESKTOP=Nuxut:Hyprland` + `nuxut-portals.conf`** (karar,
  2026-10-12); dosya sabit değil, kullanıcının Hyprland seçiminden türetilir
  (onay 2026-10-13). Kullanıcının `hyprland-portals.conf`'una dokunulmaz.
- **Gece ışığı hyprsunset süreci yerine kabuğun içinden `hyprland-ctm-control-v1`**
  (2026-10-13; ilk analizdeki "hyprsunset yönetimi" önerisinin yerine): ayrı süreç
  kendiliğinden sıfırlanmaz, `blocked` ile çakışma görünür; CTM yeteneği framework'e talep edildi.
- **Shell'e özgü olmayan yetenek framework'e:** data-control (pano), CTM (gece ışığı),
  screencopy (ileride) framework'e talep; NM istemcisi paylaşılan crate'e
  (karar bekliyor).

## Sahiplik

Yeni ikili ve modüller (`polkit/`, `portal/`, `services/screenshot`, `services/night_light`)
kabuğun; `src/main.rs`, `ipc.rs`, `hypr_config`, `install.sh`, çalışma alanı
`Cargo.toml` satırı ve hızlı panel kayıtları kabuğun;
Ayarlar sayfaları Ayarlar'ın.
