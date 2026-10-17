# Gizlilik göstergesi (`privacy`)

Durum: uygulandı (2026-10-15; P1, kullanıcı onayı).
Kod: `src/widgets/privacy.rs`, `src/services/camera.rs`, `src/app/privacy.rs`; Hyprland
`screencast` olayı `services::hyprland` (`HyprlandEvent::Screencast`), sayaç `model::Screencast`.

## Ne gösterir

Mikrofon, kamera ya da ekran yakalama kullanımdayken bar'da dolgulu bir hap; hiçbiri yokken
gizli (uzunluk 0, varsayılan bar'ın `end` başında — boşta görünmez). İkon sırası en kritik önce:
ekran (`cast`), kamera (`videocam`), mikrofon (`mic`). Ekran yakalanıyorsa dolgu `error`
(kayıt kırmızısı), yalnız mikrofon/kamera ise `primary`; ikonlar `on_error`/`on_primary`.
Palet yalnız bu iki vurgu rolünü sunuyor; macOS'un turuncu/yeşil ayrımı için framework'te rol yok,
ayrım ikonla yapılır.

İpucu: "Ekran yakalanıyor (paylaşım ya da kayıt) · başlangıç 14:02, 12 dk", "Kamera kullanımda:
zoom", "Mikrofon kullanımda: Firefox, Discord" (en çok 4 ad, fazlası "+n"). Tık: `panel`
(varsayılan sağ, ses kartı orada).

## Kaynaklar

| Kaynak | Veri | Maliyet |
|---|---|---|
| Mikrofon | `AudioState::recording` (PipeWire kayıt akışları; ses servisi zaten izliyor) | 0 |
| Ekran | Hyprland socket2 `screencast>>1\|0,SAHİP` — istemci başına başla/bitir; sayılır. Portal paylaşımı, wf-recorder, OBS hepsi buradan görünür, bu yüzden **ayrı kayıt göstergesi yok** (kabuğun kendi kaydı gelince de aynı olay). | 0 (olay) |
| Kamera | `services::camera`: `/dev` (CREATE/DELETE) ve her `video*` düğümü (OPEN/CLOSE) inotify; olaydan sonra 300 ms birleştirme, sonra `/proc/<pid>/fd` **yalnız readlink** + `/proc/<pid>/comm` | Boşta 0; tarama yalnız kamera açılıp kapanınca |

Kamera tespitinde içerik okunmaz (şart): yalnız fd bağlantısının hedefi ve süreç adı.
Başkasının süreçleri görülemez (procfs izni), kabuğun kendisi sayılmaz. İzleyici yalnız bar'da
`privacy` widget'ı varken ve `camera = true` iken çalışır.

## Güvenlik incelemesi (2026-10-16, otomatik inceleme bulguları; widgets)

İlke: şüphede gösterge AÇIK kalır (fail-closed), sönmez.

1. **Gösterge atlatma (model.rs).**
   - Eşzamanlı paylaşımlar: zaten istemci başına sayaç (Hyprland 0.56.2 `CScreenshareSession::
     screenshareEvents` oturum başına bir `1` ve bir `0` yayar; kaynak doğrulandı). Biri bitince
     diğeri sürerken gösterge sönmez (test).
   - **Bulgu (düzeltildi):** socket2 koptuğunda sayaç sıfırlanıyordu — Hyprland kuyruk taşmasında
     istemciyi düşürür, kopuş sırasında yakalama sürerken gösterge sönüyordu (fail-open). Artık
     sayaç korunur, `uncertain` işaretlenir; ipucu "bağlantı koptu, bittiği doğrulanamadı" der.
     Bedeli: kopuş sırasında biten yakalama, sayaç olaylarla 0'a inene dek göstergeyi açık tutar.
   - Sayaç kaynağı: `CScreenshareSession::screenshareEvents`
     (<https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/managers/screenshare/ScreenshareSession.cpp#L137-L154>).
   - Başlangıç durumu: Hyprland ekran paylaşımı için socket1 sorgusu sunmuyor (`hyprctl`'da yok;
     0.56.2 kaynağında `screencast` yalnız olay). Kabuk başlamadan süren yakalama bir sonraki
     başla/bitir çiftine kadar görünmez (bilinen sınır; aşağıda). "Olay tetikleyici, durum
     socket1'den" önerisi bu yüzden uygulanamadı.
   - **Kamera (düzeltildi, üç tur; karar 2026-10-16).** Üçüncü tur bulgusu "fail-open
     state drift": denge izleme öncesi açılmış okunamayan bir fd'nin kapanışıyla eksiye kırpılıp
     başka bir sürecin gerçek açmasını silebiliyordu. **Kural:** gösterge açık = denge > 0 **veya**
     okunabilir sahip var **veya** (dengesi belirsiz düğüm varken) aygıtı açabilecek okunamayan
     süreç var. Kod: `services::camera::Tracker` (IO'suz karar çekirdeği).
     (a) **Denge:** düğüm (inode) başına `IN_OPEN` − `IN_CLOSE_*` (dup/fork/SCM_RIGHTS aynı
     açıklamayı paylaşır, tek kapanış). **Üst sınırı yok** (u64, 2026-10-16 güvenlik düzeltmesi:
     eski 4096 kırpması bilgi kaybettiriyordu — sınırdan fazla açıp birkaçı dışında kapatan süreç
     dengeyi sıfıra indirir, fd'leri açıkken gösterge sönerdi; özellik testine sayaç seli eklendi).
     Savunma (2026-10-17): sayaç yine de tavana (u64) dayanırsa sarmaz; düğüme **yapışkan doygun**
     işareti konur, gösterge açık kalır ve işaret yalnız o düğüm için **yarışsız, tam ve
     şüphelisiz** bir taramada kalkar (denge o taramadan kurulur; kapanışlar, yarım/şüpheli/yarışlı
     tarama kaldırmaz).
     0'ın altına inmez; **kapanış fazlası** (0'da kapanış)
     uyarıyla loglanır ve hemen yeniden tarama tetikler. Yeni izlenen düğümün (açılış, aygıt
     yeniden takıldı) başlangıç dengesi **yarışsız** taramayla kurulur: tarama sırasında o düğüme
     olay geldiyse tarama geri çekilerek tekrarlanır; kurulurken değer max(izlemeden beri kırpılmış
     denge, taramanın bulduğu) — ikisi de gerçeğin alt sınırı. Denge **yalnız tam ve yarışsız**
     taramayla (`Scan::complete`: yarım değil, şüpheli yok; o düğüme tarama sırasında olay yok,
     kuyruk taşmadı) taramaya eşitlenir: tarama dengeyi yükseltebilir, düşürmesi yalnız taşmadan
     sonra (olaylar kesin; taramanın hiç göremediği sahip — `hidepid`, kabuğun kendisi — dengeden
     silinmez; özellik testi bunu yakaladı). Yeniden adlandırma aynı wd'yi verir, yol güncellenir.
     `/dev`'de artık olmayan düğüm izleyiciden düşer (aygıt yok = yakalama yok; taşmada kaybolan
     `IN_IGNORED` bayat dengeyle göstergeyi takılı bırakmasın — yine özellik testi).
     (b) **Tarama = atıf:** `/proc/*/fd` yalnız `readlink` + `comm`. Denge taramanın bulduğundan
     fazlaysa "Bilinmeyen uygulama".
     (c) **Okunamayan süreç (şüpheli):** `fd` dizini okunamayan sürecin `/proc/<pid>/status`
     kimliği (fsuid/fsgid/Groups; herkese okunur) düğümün mod + POSIX ACL'iyle
     (`system.posix_acl_access`) karşılaştırılır; aygıtı açabiliyorsa şüphelidir, kimliği
     okunamazsa da (fail-closed). **root tehdit modeli dışı** (kabuğu ptrace/kill edebilir,
     göstergeyi zaten aşar). Oturumun `(sd-pam)`'i muaf: adı `(sd-pam)`, ebeveyni aynı uid'in
     `systemd --user`'ı ve o ebeveynin **en erken başlamış çocuğu** ((sd-pam) ebeveyn systemd'yi
     exec etmeden önce çatallanır; adını taklit eden sonraki birim ilk çocuk olamaz). Harfiyen
     "okunamayan fd dizini varsa açık" kuralı göstergeyi hiç söndürmezdi: bu makinede 334 süreç
     (kthread hariç 24: systemd, udevd, logind, NetworkManager, Xorg, sddm, dbus-broker, polkitd,
     oomd/resolved/timesyncd, rtkit, earlyoom, dockerd + `(sd-pam)`) okunamıyor (2026-10-16 ölçüldü).
     Şüpheli **yalnız belirsiz düğüm varken** sayılır: düğümün dengesi şüpheli varken (ya da yarım
     taramayla) kurulduysa izleme öncesi okunamayan açma dengede olmayabilir; şüphelisiz tam tarama
     belirsizliği kaldırır. Kabuktan sonra başlayan tarayıcı sandbox'ının açmaları zaten dengede
     olduğundan göstergeyi boşuna yakmaz; kabuk başladığında çalışan sandbox ise o süreç çıkana
     dek "bilinmeyen" yakar (bilinçli fail-closed bedeli).
     (d) `IN_Q_OVERFLOW`: gösterge yarışsız tam taramaya dek açık; o taramada denge taramadan
     kurulur. Gösterge açıkken 5 sn'de bir doğrulama taraması; boşta yoklama yok. İzlenemeyen düğüm
     (izin) uyarıyla loglanır.
     **Testler** (`services::camera::tests`): başlamadan açık aygıt + kapanış + yeni açılış (birim
     ve gerçek inotify), yarışlı ilk tarama tekrarı, taşma (şüpheli/yarım/yarışlı taramada sürer),
     yeniden takma (birim ve gerçek inotify) + yeniden adlandırma, kapanış fazlası, şüpheli kuralı
     (mod/ACL/maske, root, sahte `(sd-pam)` ve taklitçisi), **rastgele dizi özellik testi**
     (400 tohum × 300 tur: okunabilir/okunamayan süreçler, başlangıçta açık fd'ler, tarama
     sırasında olaylar, takma/çıkarma, kuyruk taşması; gösterge gerçeği hiç eksik göstermez ve
     herkes kapatınca söner).
2. **socket2 enjeksiyonu (hyprland.rs).** Hyprland 0.56.2 `EventManager::formatEvent`
   (<https://github.com/hyprwm/Hyprland/blob/v0.56.2/src/managers/EventManager.cpp#L126-L131>) olay
   verisindeki `\n`'leri boşluğa çevirir ve veriyi 1024 baytta keser: başlık/sınıf/paylaşım adı
   yeni satır açamaz, olay adı her zaman Hyprland'ın yazdığı ilk `>>`'den öncesidir. Ek savunma:
   `parse_screencast` katı (`0|1,<rakam>`; boşluk, ek alan, eksik tür reddedilir), yalnız tam ad
   `screencast` sayılır (`screencastv2` başlık taşır, yok sayılır). `hyprctl dispatch event` gibi
   istemci kaynaklı olaylar `custom>>` adıyla gelir, `screencast` olamaz. Düşmanca test:
   `screencast_cannot_be_spoofed_from_titles` (başlıkta `screencast>>0,0`, `screencastv2`
   içinde, bozuk biçimler). osk'nin `activelayout` ayrıştırması da aynı akışta ve aynı korumayla
   güvende (düzen adı tek satır; yanlış düzen adı yalnız gösterimi etkiler, güvenlik sınırı değil).

## Bilinen sınırlar

- Kabuk başlamadan **okunamayan** bir sürecin açtığı kamera dengede yoktur; şüpheli kuralı onu
  ancak süreç aygıtı açabilecek kimlikteyse ve root değilse görür. root ve `hidepid` ile tümden
  gizlenen süreçler (procfs'te listelenmez) kapsam dışı; kabuktan sonraki açmaları yine dengede.
- Hyprland'in olay verisini satır sonundan arındırmasına güvenilir (0.56.2 doğrulandı); daha eski
  bir Hyprland'de başlıktan satır enjeksiyonu mümkün olabilir (osk `activelayout` için de aynı).
- Kabuk başlamadan süren ekran yakalaması bilinmez: Hyprland anlık durum sorgusu sunmuyor; yakalama
  bir sonraki başla/bitir çiftinde doğru görünür. socket2 koparsa sayaç korunur (fail-closed).
- Kamerayı PipeWire üzerinden kullanan uygulamada açık tutan süreç `pipewire`/`wireplumber`
  görünebilir (aygıtı PipeWire açar); asıl uygulama adı için PipeWire video akışlarını izlemek
  gerekir (ses servisinin video sınıfları — sonraki iş).
- WirePlumber/udev'in anlık yoklamaları birleştirme penceresinde kapanırsa gösterge yanmaz;
  aygıtı uzun süre açık tutan arka plan süreci varsa gösterge haklı olarak yanık kalır.
- Bu VM'de kamera yok: kamera yolu sahte `/dev` + sahte procfs ile birim/uçtan uca test edildi.

## Doğrulama

Birim: `services::camera::tests` (düğüm adları, readlink taraması, inotify ile açma/kapama ve
sonradan takılan aygıt, hızlı durma), `widgets::privacy::tests`, `model::tests::screencast_*`,
`registry`/şema testleri. Entegrasyon: `tests/it/hyprland_service.rs`
`screencast_is_forwarded_without_refresh` (sahte socket2). Canlı: izole sway + özel D-Bus/PipeWire
yok — mikrofon/ekran durumu sahte model ile birim testte; görsel kontrol izole sway'de.
