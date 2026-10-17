# Maskot Fiyu: shell yerleşimi

Bileşen framework'te: `nuxut::ui::Mascot` (`nuxut-ui` `brand` özelliği, varsayılan açık; shell
ek Cargo özelliği gerektirmez). Genel kural, renk rolleri ve boyut: framework
`docs/design/mascot.md`. Bu belge yalnızca shell'deki yerleri listeler.

## Kararlar

| Karar | Kim, ne zaman |
|---|---|
| Fiyu shell'de bildirim merkezi boş, başlatıcı sonuç yok, pano geçmişi boş ve hata (servis yok) durumlarında; "güncelleme bitti" bildiriminde `Celebrate` | kullanıcı (yerleşim onayı), 2026-10-13 |
| ASLA: bar, kilit ekranı (`nuxut-lock`), parola/kimlik doğrulama (`nuxut-polkit`, PAM), uygulama ikonu | kullanıcı (mascot.md), 2026-10-13 |
| Boş durumlar framework `nuxut::ui::EmptyState` ile çizilir (shell'deki geçici `widgets/empty_state.rs` kaldırıldı). Maskot gizliyken (`framework.toml [appearance] mascot = false`) ya da alana sığmadığında maskot yer kaplamaz, metin kendi başına ortalı | framework bileşeni kaynak, 2026-10-14; uygulama |
| **Gizliyken eski görünüm** (önceki satırı daraltır): `mascot = false` iken bildirim merkezi boş durumu eski sessiz ikonuna döner — `EmptyState::with_fallback_icon(theme, Icon::named(IconName::NotificationsOff))` (fw 5fe1e2e; ikon ölçeklenmez, sığmazsa düşer). Başlatıcı/pano eski gizli görünümü ikonsuzdu (yalnız metin) → değişmez. Yalnız gizlemede; maskot açıkken sığmazsa ikon yok, yalnız metin | 2026-10-14; uygulama |
| Sürüm artışı yalnız sürüm yayınında (`Cargo.toml` `version`, release akışı); test döneminde `0.1.0` kalır → "güncelleme bitti" test döneminde görünmez, kabul; install.sh'ye bir şey eklenmez | 2026-10-13 |
| Hoş geldin gösterilecekse "güncelleme bitti" bildirimi BASTIRILIR — ikisi asla birlikte çıkmaz; `last_version` yine yazılır | 2026-10-13 (uygulama) |
| "Güncelleme bitti": install.sh sinyali yok; kabuk açılışta `shell-state.toml` `last_version` ile kendi sürümünü karşılaştırır, artışta iç bildirim bir kez + `Celebrate`; ilk kurulum, aynı/düşük sürüm, bozuk dosya bildirimsiz; maskot alanı yalnız iç bildirimlerde (D-Bus ayarlayamaz); kart maskotu canlı çizer | 2026-10-13 (uygulama) |

## Yerleşim listesi

| Yüzey | Durum | `Mood` | Boyut | Kod |
|---|---|---|---|---|
| Bildirim merkezi (kenar paneli) | Geçmiş boş | `Empty` | 96 (alan yetmezse 48'e dek küçülür, 48 de sığmazsa yok) | `widgets/notifications_panel.rs` (`EmptyState`) |
| Başlatıcı, uygulama kipi | Aramada sonuç yok | `NoResults` | 96 (aynı sığdırma) | `launcher_view.rs` (`EmptyState`), `empty_mood` |
| Başlatıcı, pano kipi | Geçmiş boş | `Empty` | 96 | aynı |
| Başlatıcı, pano kipi | Pano geçmişi kapalı (`[clipboard] enabled = false`) | `Empty` | 96 | aynı |
| Başlatıcı, pano kipi | İzleyici yok (data-control protokolü yok): "servis ulaşılamadı" | `Error` | 96 | aynı |
| Başlatıcı, pano kipi | Süzgeçte eşleşme yok | `NoResults` | 96 | aynı |
| Bildirim kartı (popup + merkez) | "Güncelleme bitti" iç bildirimi (`InternalNotice::with_mascot`) | `Celebrate` | 64 (framework kart maskot yuvası, metnin solunda, karosuz) | `widgets/notification_cards.rs` `card_mascot`, `build_card`; gönderen `app/update_notice.rs` |
| Dosya seçici (`nuxut-filechooser`) | Klasör boş / aramada sonuç yok / klasör okunamıyor (izin yok, yok, hata) | `Empty` / `NoResults` / `Error` | 96 (aynı sığdırma); gizliyken yedek ikon `FolderOpen` / `Search` / `Lock`-`Warning` | `filechooser_view.rs` `update_empty` (görev, 2026-10-15) |

Yerleşim: framework `nuxut::ui::EmptyState` (`EmptyState::new(theme, metin).with_mascot(theme,
Mood::…)`; kural framework `docs/design/mascot.md` "Boş durum yerleşimi"). Maskot + başlık tek
sütun, liste/görünüm alanında (`set_rect`) yatay ve dikey ortalı; kenarlardan `space(4)` dolgu,
maskot–metin aralığı `space(4)`, metin `Title` rolü. Sığdırma framework'te (`fit_art`): kenar =
min(96, alan − aralık − metin), 48'in altında maskot çizilmez (yüz okunmaz). `mascot = false`:
maskot yer kaplamaz, metin tam ortada (sonuç `EmptyLayout::art == None`); bildirim merkezi
istisna: yedek ikon `NotificationsOff` + metin ("gizliyken eski görünüm", 2026-10-14).
Tema/kip/`[appearance]` değişimi `set_theme` ile canlı. Başlatıcıda ruh hâli `set_mood`, metin
`set_title`; dil değişiminde bileşen yeniden kurulur, son ruh hâli korunur.

### Kapsam dışı bırakılanlar (gerekçe)

- **Takvim popout'u / Takvim kartı** "Takvim servisi çalışmıyor", "Bugün etkinlik yok": liste tek
  satırlık sabit yuvada; 96 px maskot popout boyunu büyütür, kart başka kartlarla yer paylaşır.
  Kompakt satır durumu, kart/boş-sayfa değil.
- **Hızlı ayar kartları** (Wi-Fi "NetworkManager çalışmıyor", Bluetooth "BlueZ çalışmıyor"): kart
  alt metni; maskot sürekli görünen kenar paneli kartında dikkat dağıtır.
- **Popup bildirimleri, bar ipuçları**: bar ve sürekli yüzey kuralı.

### "Güncelleme bitti" (`Celebrate`)

**Karar (2026-10-13):** install.sh sinyali YOK. Kabuk açılışta kendi sürümünü
(`CARGO_PKG_VERSION`) `$XDG_STATE_HOME/nuxut/shell-state.toml` içindeki `last_version` ile
SemVer önceliğiyle karşılaştırır; arttıysa kendi iç bildirimini (`InternalNotice`, uygulama adı
"Nuxut") bir kez gösterir, kartta `Celebrate`. `[appearance] mascot = false` iken maskotsuz
normal kart. İlk kurulumda (dosya yok) gösterilmez, yalnızca `last_version` yazılır (Hoş geldin
var). Sürüm düşerse/aynıysa gösterilmez, `last_version` güncellenir. Bozuk dosya: panik yok,
uyarı + doğru değerle üzerine yazım.

Uygulama (2026-10-13):

- Durum: `services::shell_state` — dosya yeni (önceden kabuk durum dosyası yoktu), donuk v1
  `last_version = "<semver>"`, okuma 4 KiB, sürüm metni ≤ 64 bayt, bilinmeyen anahtar yok sayılır;
  yazım `nuxut-apply` (`apply::write_own`, atomik, ortam kapısı), dizin 0700. Karşılaştırma
  `semver::Version::cmp_precedence` (ön sürüm < sürüm; `+derleme` meta verisi yok sayılır).
- Bildirim YALNIZCA yeni değer diske yazıldıysa gösterilir: yazamayan örnek (kilitsiz ikinci
  örnek; ev dizinine yazamayan test örneği) hiç göstermez — her açılışta tekrar etmez.
- Açılış: `app/update_notice.rs` `check_update_notice`, `start`'ta Hoş geldin'den sonra. Popup'lı
  (DND ve `[notifications.apps."Nuxut"]` kuralı geçerli), düğmesiz. Metin: `shell-update-done-title`
  ("Nuxut güncellendi" / "Nuxut updated"), `shell-update-done-body` ("Yeni sürüm: { $version }").
- Model: `InternalNotice::with_mascot(NoticeMood)` → `Notification::mascot`. `NoticeMood`
  yalnızca onaylı bildirim yerleşimlerini içerir (bugün `Celebrate`). Model piksel değil ruh hâli
  taşır. **Dış D-Bus bildirimleri bu alanı ayarlayamaz:** sunucu (`build_notification`) hep
  `None` kurar, hiçbir hint okunmaz; `NotificationCenter::on_event(Added)` alanı ayrıca siler
  (testler: `tests/notifications_dbus.rs` `dbus_client_cannot_set_mascot`,
  `tests/it/notifications_model.rs` `external_event_cannot_set_mascot`).
- Kart (`widgets/notification_cards.rs`): framework `NotificationCard` maskot yuvası (fw 1c9174a;
  `NotificationCard::with_mascot(theme, Mood::Celebrate)`, `build_card`/`card_mascot`). Maskot metin
  sütununun solunda, karosuz, `NOTIFICATION_MASCOT_SIZE` (64 px); kartın kaydırma/kapanma
  solmasına kart kendisi uygular. Kart maskot isteğini `[appearance] mascot`'tan bağımsız taşır;
  `mascot = false` iken yuvayı kart kapatır (metin sola yaslanır, kart kısalır). Tema değişiminde
  kart yeniden kurulmaz: `CardSet::set_theme` → `card.set_theme` + ölçü düşer; sonraki karede
  `prepare` → `measure`, `draw` → `set_rect` yeni yüksekliği dizer, yığın kayar. Popup ve
  bildirim merkezi aynı `CardSet`'i kullanır. Maskotlu kartta içerik görseli yok (model atar;
  tek görsel kuralı).
- Geçici yol kaldırıldı (2026-10-14; talep, fw 1c9174a): eskiden
  maskot kartın sağdaki 56 px görsel yuvasına, `surface_bright` karo üstünde, şeffaf 1×1 yer
  tutucu dokuyla ve shell'de kopyalanmış kaydırma solması kuralıyla çiziliyordu; hepsi kalktı.
- Bilinen sınır: test döneminde `Cargo.toml` sürümü güncellemelerde artmıyor (hep `0.1.0`);
  bildirim ancak sürüm artırılınca görünür. Git/derleme kimliği yok (sıralanamaz, "arttı"
  tanımsız). `shell-state.toml` install.sh `--reset-config` taşıma listesinde DEĞİLDİR ve
  girmemelidir (girerse sıfırlamadan sonra bildirim hiç çıkmaz).

**Karar (2026-10-13) — sürüm artışı:** sürüm yalnız sürüm yayınında artar
(`Cargo.toml` `version`, release akışı). Test döneminde `0.1.0` kalır; bildirim test döneminde
görünmez — kabul edildi. install.sh'ye sinyal ya da sürüm adımı EKLENMEZ.

**Karar (2026-10-13) — Hoş geldin ile birlikte çıkmaz:** açılışta Hoş geldin
gösterilecekse güncelleme bildirimi BASTIRILIR; `last_version` yine yazılır (sonraki açılışta da
çıkmaz). Uygulama (2026-10-14): `start` önce `offer_welcome()` çağırır; dönüşü
"Hoş geldin ekranda" — `setup.toml` yok ve `should_offer_welcome` (ayar açık, Hyprland oturumu,
yazabilen varsayılan örnek) doğruysa ve pencere bu çağrıda başlatıldıysa YA DA bu oturumda zaten
başlatılmışsa (`welcome-<imza>` işareti) `true`. `nuxut-settings` bulunamaz ya da başlatılamazsa
`false` (Hoş geldin yok → bildirim normal kurala göre). Sonra `check_update_notice(cx, welcome)`
→ `update_check(welcome)`: `shell_state::check` her durumda çalışır (yazım), `welcome` iken
bildirim yok. Test: `app::update_notice::welcome_suppresses_notice_but_records_version`.

## Bekçi

`tests/it/mascot_placement.rs`: maskot tanımlayıcıları (`Mascot`, `Mood::`, `mascot_image`,
`mascot_colors`) yalnızca izin listesindeki dosyalarda (`notifications_panel.rs`,
`launcher_view.rs`; "güncelleme bitti" için `services/notifications/internal.rs` (`NoticeMood`),
`notification_cards.rs`, `app/update_notice.rs`); `lock/`, `polkit/`, `crates/nuxut-pam/` ve bar dosyaları ayrıca yasak listede.
Yeni yerleşim önce bu belgeye, sonra izin listesine yazılır.

## Doğrulama (2026-10-13)

- Birim: `empty_state` (ortalama, gizlide aralıksız, küçülme/düşme), bildirim merkezi (Empty, 96,
  ortalı; canlı gizleme → ölçü 0, görsel yok, yedek ikon; kısa panelde küçülme/düşme), başlatıcı
  (her durumda mood; 96 ortalı; gizleme; dolu listede maskot yok).
- Canlı: izole headless sway, framework sandbox'ı; bildirim merkezi boş,
  başlatıcı sonuç yok, pano boş — koyu, açık ve `mascot = false` koyu.

## Doğrulama (2026-10-14 — framework `EmptyState`'e geçiş)

- Birim: bildirim merkezi (96 ortalı, `EmptyState` alanı = görünüm alanı; canlı gizleme → görsel
  yok, `art == None`, metin sütunu görünüm alanının ortasında; kısa panelde küçülme/düşme),
  başlatıcı (her durumda mood; 96 ortalı; gizleme; dolu listede maskot yok), Hoş geldin bastırma.
  Geometri testleri framework `empty_state.rs`'te.

## Doğrulama (2026-10-13 — "Güncelleme bitti")

- Birim: `services::shell_state` (ayrıştırma/sınır/bozuk, karar kuralları, diskte akış: ilk kurulum
  gösterilmez, artış bir kez, ikinci açılış göstermez, aynı/düşüş gösterilmez ve yazılır, bozuk
  onarılır, geçici dosya kalmaz, yazamayan örnek göstermez), `app::update_notice` (metin + mood,
  Shell üzerinden aynı akış), `notification_cards` (`mascot_mood` gizleme, aç/kapa kartı yeniden
  kurar, kaydırma solması), model (maskot taşınır, görsel atılır; dış olay maskot kuramaz),
  D-Bus uçtan uca (hint/kategori/görsel/uygulama adı ile maskot kurulamaz).
- Canlı: izole headless sway, `NUXUT_SHELL_INSTANCE=test-celebrate`, geçici XDG_*, durum dosyasına
  `0.0.1` yazılıp başlatıldı: popup "Nuxut güncellendi / Yeni sürüm: 0.1.0" + Fiyu (koyu, açık),
  `mascot = false` koyuda maskotsuz kart; çıkışta dosya `last_version = "0.1.0"`.

## Doğrulama (2026-10-14 — framework kart maskot yuvası, fw 1c9174a)

- Birim: `notification_cards` `mascot_only_for_internal_request` (istemeyen/dış kartta yuva yok),
  `mascot_toggle_relayouts_without_rebuild` (açıkken 64 px sol yuva; canlı gizlemede kart
  korunur, yuva kalkar, kart kısalır; geri açılınca aynı ölçü; maskotsuz kart etkilenmez). Dış
  bildirim güvenceleri (`dbus_client_cannot_set_mascot`, `external_event_cannot_set_mascot`)
  değişmeden geçer.
- Canlı: izole headless sway, `NUXUT_SHELL_INSTANCE=test-notifmascot`, durum dosyasına `0.0.1`:
  popup ve bildirim merkezinde Fiyu metnin solunda, karosuz (koyu, açık); `mascot = false`
  açılışta maskotsuz kısa kart; koyuda `framework.toml` canlı değiştirilince merkezdeki kart
  maskotsuz ve kısa yeniden dizildi.
