# VM pano köprüsü (X11 ↔ Wayland)

Durum: uygulandı, 2026-10-16. İstek: kullanıcı (2026-10-16).
Kullanıcı ilkesi: "Nuxut yapabiliyorsa varsayılan o; devralma
yedekli ve geri alınabilir". Araştırma: `docs/research/vm-clipboard.md`.
Kod: `src/services/clipboard/x11/` (`policy.rs` saf kararlar, `conn.rs` X11 tarafı, `mod.rs`
thread ve algılama), bağlantı `src/app/clipboard_history.rs`.

## Sorun ve neden

VM konuk ajanları (spice-vdagent, open-vm-tools `vmtoolsd`, VirtualBox `VBoxClient`) panoyu
yalnızca X11 seçimleriyle konuşur. Hyprland'in XWM'i X11 ↔ Wayland pano eşitlemesini iki yönde
de **klavye odağı bir X11 penceresindeyken** yapar (bilinçli gizlilik koruması). Ajanın penceresi
olmadığı için VM'de pano hiç akmaz. Mutter odak koşulu olmadan eşitler (GNOME'da çalışır), KWin
Wayland → X11 yönünde odak ister.

## Kapsam

- Yalnızca `CLIPBOARD` seçimi (birincil seçim taşınmaz: ajanlar da ana makineye yalnız panoyu
  taşır; seç-bırak gürültüsü).
- Türler: metin (UTF-8; X11'de `UTF8_STRING`, `STRING` Latin-1 çevrimi, `TEXT`), tek görsel
  (`image/png` önce, yoksa ilk `image/*`), en çok 4 ek MIME benzeri tür (ör. `text/html`,
  `text/uri-list`). Wayland → X11 yönünde teklifin tüm türleri tembel sunulur.
- Boyut: yön başına toplam 16 MiB; X11'e tek `ChangeProperty` ile yazılır (INCR gönderimi yok,
  bağlantının en büyük isteğine sığmayan reddedilir; Xwayland BIG-REQUESTS ile ~16 MiB). X11'den
  okumada INCR desteklenir.

## Ne zaman devrede (`[clipboard] x11_bridge`)

| Değer | Davranış |
|---|---|
| `"auto"` (varsayılan) | `/proc/cpuinfo` `hypervisor` bayrağı (x86 dışında atlanır) **ve** bir konuk ajanı süreci (`/proc/*/comm`: `spice-vdagent`, `vmtoolsd`, `vmware-user`, `VBoxClient`). Ajan oturumla başladığı için başlangıçtan sonra 0, 2, 5, 10, 20, 40, 80, 160, 300 sn'de taranır, sonra tarama durur. Bulununca oturum boyunca açık kalır. |
| `"on"` | Her zaman (ör. X11 pano araçları). |
| `"off"` | Hiç. |

Gerekçe: köprü, Hyprland'in "odakta olmayan X11 istemcisi panoyu okuyamaz/yazamaz" korumasını
kaldırır (her X11 istemcisi panoyu okuyabilir). VM'de bu zaten istenen davranıştır (ajan
odaksızdır); VM dışında koruma korunur. Görevdeki "XWayland panosu eşitlenmiyorsa" koşulu: kabuk
yalnızca Hyprland'de çalışır ve Hyprland'in eşitlemesi her zaman odak korumalıdır; odaksız
eşitlemenin gerekip gerekmediğini çalışma anında ölçmenin güvenilir yolu yok, gereken tek yaygın
durum VM ajanıdır. DISPLAY yoksa (Xwayland kapalı) köprü hiçbir şey yapmaz.

Geçmiş kapalıyken (`enabled = false`) de köprü devreye girince data-control izlemesi başlar
(birincil seçim izlenmez); geçmiş filtresi yine `Disabled` ile hiçbir şey okumaz.

## Akış (olay tabanlı, boşta sıfır iş)

Tek thread `nuxut-x11-clip`; X11 soketi ve uyandırma soketi birlikte `poll` edilir.

- **X11 → Wayland.** XFixes `SelectionNotify` (sahibi değişti) → `TARGETS` → plan
  (`policy::plan_fetch`) → türler sırayla dönüştürülür (tek dönüşüm 2 sn, toplam 5 sn) →
  `DataControlHandle::set_selection` (framework data-control kaynağı; içerik + `application/x-nuxut-x11-bridge`
  işareti).
- **Wayland → X11.** Framework data-control teklifi (yalnız pano) → `policy::decide_wayland` →
  X11 `CLIPBOARD`'ı kendi görünmez penceremizle alırız, hedefleri (`TARGETS`, `TIMESTAMP`,
  teklifin türleri + X11 metin adları + işaret) **tembel** sunarız: içerik yalnızca bir X11
  istemcisi istediğinde `DataControlHandle::read` ile okunur (son okuma aynı istek için önbellekte).

## Döngü önleme

1. **İşaret türü** `application/x-nuxut-x11-bridge`: X11 → Wayland seçimlerimizde ve Wayland → X11
   hedeflerimizde. Wayland teklifinde görülürse (kendi koyduğumuz) X11'e geri taşınmaz; X11 hedef
   listesinde ya da kabuğun süreç işaretinde (`nuxut::owner_mime`) görülürse Wayland'e taşınmaz.
2. **Kendi penceremiz**: sahibi bizsek XFixes olayı yalnızca zaman damgasını günceller.
3. **Compositor'ın XWM'i**: pano sahibi `WM_S<n>` sahibiyle aynı X11 istemcisiyse (kaynak kimliği
   taban bitleri) compositor Wayland seçimini kendisi yansıtmıştır → taşınmaz.
4. **Compositor'ın X11 → Wayland yansıtması** (odakta X11 penceresi varken Hyprland da eşitler):
   gerçek bir X11 sahibi değişiminden sonra 1 sn içinde gelen, türleri o sahibin hedeflerinin alt
   kümesi olan işaretsiz Wayland teklifi X11'e geri taşınmaz (aksi halde ajanın sahipliğini
   alıp ana makineye "konuk kopyaladı" dedirtirdik). Bilinen sınır: kullanıcı 1 sn içinde aynı
   türlerle bir Wayland uygulamasında kopyalarsa o kopya X11'e taşınmaz (bir sonraki kopyada düzelir).
5. **Aynı içerik**: X11'den gelen içerik son koyduğumuzla aynıysa (özet) ve arada Wayland'de
   başka seçim olmadıysa yeniden konmaz.

## Yetki (güvenlik incelemesi "authorization-bypass", 2026-10-16)

İlk sürüm, köprü devredeyken Hyprland'in "yalnız odaktaki X11 istemcisi panoya erişir" kuralını
**her** X11 istemcisi için kaldırıyordu. `auto` yalnızca konuk ajanı yüzünden açıldığı hâlde
odaksız herhangi bir X11 uygulaması panoyu okuyabiliyor ve Wayland panosuna seçim koyabiliyordu;
konuk ajanı adı (`comm`) başka bir yerel kullanıcının süreciyle de eşleşiyordu; `DISPLAY` her
biçimde (TCP / SSH yönlendirmesi dahil) kabul ediliyordu. Kapatılan yollar:

- **Yalnız kendi X sunucumuz:** `DISPLAY` yalnız yerel biçimde (`:N`, `unix:N`;
  `policy::local_display`); bağlanınca soketin karşı ucunun uid'i (`SO_PEERCRED`) bizimki değilse
  bağlantı kurulmaz.
- **İstemci yetkisi (`auto`), karar 2026-10-16: yalnız doğrulanmış konuk ajanı.** Seçim isteği
  (tür listesi dahil) ve X11'de pano sahipliği yalnızca isteyen/sahip **penceresinin** istemci
  süreci (XRes `QueryClientIds` → yerel PID) bu kullanıcının konuk ajanıysa (`/proc/<pid>` sahibi
  + `comm`) geçerli; diğerleri reddedilir (`policy::client_allowed`). Yalnız "ajan çalışıyor"
  yetmez, pencere ajana ait olmalı: yanıt yalnız ajanın penceresine yazılır. PID okunamazsa yetki
  yok (kapalı-güvenli). **Odak temelli izin yok:** `ConvertSelection`'da isteyen pencereyi istemci
  kendisi seçer; odaksız bir istemci odaktaki pencereyi isteyen gösterip yanıtı o pencerenin
  özelliğinden okuyabiliyordu (X11'de her istemci her pencerenin özelliğini okur) — odak isteği
  gönderene bağlanamaz. Önceki iki düzeltme (X11 `GetInputFocus`, sonra compositor odağı) bu
  yüzden kaldırıldı. Odaktaki normal X11 uygulamalarının panosunu Hyprland'in kendi XWM
  eşitlemesi taşır; köprünün varlık nedeni VM ajanıdır.
- **Sunucu:** bağlanılan soketin karşı ucu bu kullanıcının **Xwayland**'i olmalı
  (`SO_PEERCRED` uid + `/proc/<pid>/comm`; `policy::is_own_xwayland`): aynı kullanıcının Xvnc /
  Xpra / Xephyr gibi panoyu ağa ya da başka ekrana taşıyabilen sunucusuna da bağlanılmaz.
  `XAUTHORITY` x11rb'nin olağan çerezidir; kimlik denetimi çereze değil soket kimliğine dayanır.
- **İstek doğrulama** (`policy::check_request`, sıra bağlayıcı): (1) oturum kilidi **her istek
  anında ilk** bakılır — kilitten önce yapılmış teklif de kilitliyken verilmez, tür listesi dahil;
  önbellekteki içerik sıfırlanarak bırakılır; Wayland okuması sürerken kilitlenirse de verilmez.
  (2) Yanıt penceresi kök, kendi penceremiz ya da yok olamaz. (3) Yanıt özelliği pencere
  yöneticisi/masaüstü/kaynak özelliği olamaz (`WM_*`, `_NET_*`, `RESOURCE_MANAGER`,
  `CUT_BUFFER*`…; `policy::property_allowed`) — köprü başkası adına bu özellikleri yazan bir araç
  olmaz. (4) İstemci yetkisi. INCR gönderimi yok: tek isteğe (`maximum_request_bytes`) ve 16 MiB'e
  sığmayan içerik reddedilir; aynı teklif/tür önbellekten verilir (tekrarlı istek Wayland'i
  yeniden okumaz). Hariç uygulama ve hassaslık teklif anında (kopyalama kaynağına göre) karar
  verilir: o teklif için X11 sahipliği hiç alınmaz, istek gelemez.
- **Yanıt özelliği silinir:** ajanın penceresine yazılan içerik ajan okuyup silmezse 2 sn sonra
  köprü siler (`policy::ReplyCleanup`; en çok 16 bekleyen kayıt).
- Kalan risk (kabul, 2026-10-16): yetkisiz bir istemci **ajanın** penceresini isteyen gösterirse
  yanıt ajanın penceresine yazılır ve X11 o istemcinin bu özelliği okumasını engellemez. Pencere
  dar: yalnız Nuxut X11 panosunu tutarken (kullanıcı Wayland'de kopyaladıktan sonra), saldırgan
  isteği yapıp ajan özelliği okuyup silmeden (ya da 2 sn dolmadan) okumalı; saldırganın ajanın
  pencere kimliğini bilmesi gerekir. Odak kandırmacasından (her an, her pencereyle) çok daha dar;
  tam kapatmak X11 yalıtımı (ayrı Xwayland / güvenlik uzantısı) ister.
- **Ajan algılama:** yalnız bu kullanıcıya ait süreçler (`policy::is_own_guest_agent`).
- `on` kullanıcının açık seçimidir: koruma herkes için kalkar (ayar açıklaması söyler).
- Bilinen sınır: bağlandıktan sonra çatallanan bir ajan (bağlantıyı açan PID çıkmışsa) ajan
  sayılmaz; spice-vdagent (`-x` ya da önce çatallanan varsayılan), vmtoolsd, VBoxClient bağlanmadan
  çatallanır. Aynı süreçteki her X11 bağlantısı aynı PID'i taşır (ajan süreci içindeki her pencere yetkili).
- Testler: `policy` birim (yerel display, ajan sahibi, yalnız-ajan yetki tablosu, yanıt silme takvimi); canlı (izole sway + kendi
  Xwayland, `auto`): sıradan xclip panoya koyamaz/okuyamaz, `spice-vdagent` adlı kendi süreç
  koyar/okur, hassas teklif ajana da verilmez (5/5).

## Gizlilik

- Hassas teklif **iki yönde de taşınmaz ve okunmaz**: Wayland'de framework `sensitive` (parola
  yöneticisi ipucu `x-kde-passwordManagerHint`, adında `secret`/`password`) ya da kırpılmış liste
  → X11'deki eski seçimimiz bırakılır; X11 hedeflerinde aynı adlar → okunmaz. Karar yalnızca
  adlara bakar. Sonuç: KeePassXC'den kopyalanan parola ana makineye gitmez (bilinçli; GNOME'dan
  farklı). Gerekirse ileride ayrı bir anahtar.
- **Gizlilik kapısı (`Gate`) iki yönde de uygulanır** (güvenlik incelemesi düzeltmesi,
  2026-10-16). Önceki metin "kilitliyken framework okuma ve seçim koymayı reddeder" diyordu;
  yanlıştı: framework kuralı yalnız kilidi tutan süreçte geçerlidir, kilit ekranı ayrı süreçtir
  (nuxut-lock), kabukta hiç devreye girmez. Artık köprü geçmişle aynı kapıya bakar (logind
  `LockedHint` + etkin pencere sınıfı + `exclude_apps`):
  - Wayland → X11: kopyalama anında kilitli ya da hariç tutulan uygulama etkinse teklif X11'e
    taşınmaz (eski seçimimiz bırakılır); kilitliyken X11 isteklerine içerik verilmez.
  - X11 → Wayland: kilitliyken ya da hariç tutulan uygulama etkinken X11 seçimi okunmaz (hedef
    listesi dahil); okuma sürerken kilitlenirse Wayland'e konmaz. Odaktaki hariç tutulan X11
    uygulamasının kopyası Wayland uygulamalarına Hyprland'in kendi eşitlemesiyle yine geçer.
  - Gerekçe (`exclude_apps` köprüde de): o liste parola ipucu koymayan parola yöneticileri
    içindir; ipuçlu parola ana makineye gitmiyorsa ipucusuzu da gitmemeli.
- Hassas ad kuralı tek yerde: `nuxut::is_sensitive_mime` (framework işareti, köprü, geçmiş
  filtresi); kopyalar ayrışırsa bir yol hassas teklifi kaçırırdı.
- Köprünün X11'den getirdiği seçim pano geçmişine girer (ana makineden kopyalanan da geçmişte
  görünsün): geçmiş servisi bu işaretli kendi teklifini `own` saymaz, aynı gizlilik
  filtresinden geçirir.
- İçerik loglanmaz; tamponlar `Zeroizing`, INCR büyürken eski tampon sıfırlanır.

## Dayanıklılık

- Bilinen sınır (canlı test, 2026-10-16): kabuk açılırken data-control izlemesi bağlanmadan
  (≈ 1 sn) X11'de kopyalanan seçim Wayland'e konamaz ("pano protokolü yok") ve yeniden
  denenmez; bir sonraki X11 kopyasında düzelir. Soğuk açılışta bir kez görüldü.

- Dış girdi sınırlı: hedef listesi ≤ 64 (fazlası tümüyle atlanır), ad ≤ 256 bayt ve UTF-8,
  türler toplam 16 MiB, ertelenen olay kuyruğu 256, atom önbelleği 4096.
- X11 protokol hatası (istekçinin penceresi kayboldu vb.) yalnızca o isteği düşürür; bağlantı
  hatası oturumu bitirir, 1 sn → 5 dk geri çekilmeyle yeniden bağlanılır (60 sn kararlı çalışma
  sayacı sıfırlar). Son Wayland teklifi yeniden bağlanınca yeniden sunulur. Panik yok.
- Kip değişince eski köprü durur (X11 seçimi bırakılır), yenisi kurulur.

## Kullanıcı betiğinin devralınması

Karar (2026-10-16): kabuk devralır, **çalışma anında**. Kod `services/clipboard/takeover.rs`.

- **Neden.** Kullanıcının `~/hypr-bridge.sh`'ı (`exec-once = ~/hypr-bridge.sh`: `wl-paste
  --type text --watch xclip` + 0,1 sn `xclip -o` yoklaması) Nuxut köprüsüyle birlikte çalışırsa iki
  köprü birbirini besler. Daha ciddisi: bu betik hassas teklifi yok sayar (`wl-paste --watch`
  hassas teklifte `CLIPBOARD_STATE=sensitive` verir, betik bakmaz) — KeePassXC'den kopyalanan
  parola X11 üzerinden konuk ajanına, oradan **ana makineye** gider. Nuxut köprüsü taşımaz.
- **Ne zaman.** Kurulumda değil; yalnızca Nuxut köprüsü gerçekten çalışırken: `x11_bridge`
  kapalı değil + köprü Xwayland'e bağlandı (`BridgeEvent::Connected`) + data-control izlemesi
  bağlı. Böylece kullanıcı hiçbir girişte panosuz kalmaz. Süreç başına bir kez, thread'de.
- **Ne.** Hyprland ayar zinciri (hyprlang; `hypr_config::parse::scan_chain`) taranır; komutu ya
  da çalıştırdığı betik (≤ 64 KiB, yorum dışı) hem `wl-paste` hem `xclip`/`xsel` içeren
  `exec-once`/`exec` satırları tek `nuxut-apply` işlemiyle (`hypr.clipboard_bridge` snapshot'ı)
  kaldırılır ve aynı dosyaya `# >>> nuxut:clipboard-bridge` bloğu içinde yorumlanmış hâli + gerekçe
  yazılır. Betik silinmez; çalışan betik sürecine dokunulmaz (kill/sinyal yok) — bir sonraki
  girişte satır çalışmaz. Lua ayarında yapılmaz (bilinen sınır).
- **Geri alma.** `nuxut-shell msg hypr undo` (eşitleme kapalıyken de) devralma snapshot'ını
  geri yükler: satır geri gelir, blok kalkar; `msg snapshot restore <id>` de çalışır. İşaret
  (`$XDG_STATE_HOME/nuxut/clipboard-bridge-takeover`, snapshot kimliği) kalır: geri alınan devralma
  kendiliğinden tekrarlanmaz.
- **Testler.** Saf tarama (betik/satır içi köprü, yorumlu satır, `cliphist` gibi benzerler
  sayılmaz) ve geçici ev dizininde gerçek devralma → tekrar yok → geri alma → yeniden devralma yok.

## Nasıl denenir (gerçek VM, kullanıcıyla)

1. `~/.config/nuxut/shell.toml` → `[clipboard] x11_bridge = "auto"` (varsayılan) ve
   kabuğu başlat; köprü bağlanınca `hypr-bridge.sh` satırı kendiliğinden yedekli kapatılır.
2. Log: `journalctl --user -t nuxut-shell` ya da kabuk logunda "pano köprüsü: VM konuk ajanı
   bulundu" ve "Xwayland'e bağlandı".
3. Ana makinede metin kopyala → konukta bir Wayland uygulamasına (ör. foot, Nuxut başlatıcı)
   yapıştır. Ana makinede ekran görüntüsü kopyala → konukta görsel yapıştır.
4. Konukta Wayland uygulamasında kopyala → ana makinede yapıştır (metin ve görsel).
5. KeePassXC'den parola kopyala → ana makineye **gitmemeli** (bilinçli).
6. `nuxut-shell msg clipboard list`: ana makineden gelen öğeler geçmişte.
7. Sorun olursa `x11_bridge = "off"` ve betik satırının başındaki `#` kaldırılarak eski hâle dönülür.

## Test

- Birim (`policy.rs`): hipervizör bayrağı, ajan adları, kip ayrıştırma, okuma planı (tercih,
  işaret, hassas, bozuk, ek tür sınırı), X11 hedefleri (metin adları, işaret), Latin-1 çevrimi,
  compositor istemci tabanı, Wayland kararları (işaret, hassas, kırpılmış, yansıtma penceresi),
  özet.
- Uygulama notu: sahipliği kaybettiğimizi yalnız `SelectionClear` bildirir. XFixes sahip
  olayı biz sahiplik aldıktan sonra işlenebilir (önceki sahibin kapanması kuyrukta); bu yüzden
  olay `owned`'a dokunmaz ve şu anki sahip olaydaki sahip değilse okunmaz (canlı testte
  yakalandı: önceki X11 sahibi kapanırken yapılan Wayland kopyası X11'e geçmiyordu).
- Canlı test güvenliği (2026-10-16 olayı): `Xwayland -displayfd` kullanıcının Hyprland Xwayland'i
  :1'i seçip dinleyemedi ("server already running"), test istemcileri ~3 sn kullanıcının
  Xwayland'ine bağlandı ve kullanıcının X11 panosuna test metni yazıldı. Test betiği artık kilit
  dosyası ve soketi olmayan sabit yüksek display (:90–97) seçer, `/tmp/.X<N>-lock` PID'i kendi
  Xwayland'imiz değilse hiçbir X11 istemcisi başlatmaz.
- Canlı (izole sway, kendi rootless Xwayland'imiz — sway'in XWM'i kapalı, yani Hyprland'in odaksız
  durumu gibi hiç eşitleme yok; kendi `xclip` süreçlerimiz, `wl-copy`/`wl-paste` test
  display'inde; kullanıcının panosu yok): iki yönde metin ve PNG, işaretin geri yansımaması,
  `x-kde-passwordManagerHint` teklifinin X11'e taşınmaması, geçmişe girme.

Sonuç (2026-10-16, izole sway 1.12 + Xwayland 24.1.14 :97, `x11_bridge = "on"`): 12/12 geçti —
X11 → Wayland metin ve PNG (işaret var, X11 sahibi xclip olarak kaldı), Wayland → X11 metin
(`UTF8_STRING`, `STRING`), `TARGETS`'ta işaret, PNG bayt eşit, Wayland seçimi wl-copy'de kaldı,
yalnız parola ipucu sunan teklif X11'e taşınmadı (eski seçim bırakıldı), X11'den gelen metin
`msg clipboard list`'te. Betik: `flock -o` içinde, test-compositor `--owner`, `dbus-run-session`.
