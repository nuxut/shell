# Kenar panelleri: içerik listesi, sol panel, sabitleme

Kullanıcı yorumu 6 (2026-10-10): "Panellerin de içerikleri eklenip, sıralanıp değiştirilebilmeli
tıpkı bar gibi. Ve sola da panel ekleyebilelim. Panellerin (hangi taraftan çıkıyorsa o köşesine,
bar üstte/altta ise ona yakın köşede) sabitleme tuşu olabilir. Basınca panel kaybolmasın."

## 1. İçerik listesi (zaten var, sözleşme olarak sabitlendi)

Bar'ın modeliyle aynı: `[panels.<kenar>] widgets = [...]`, **sıra = görünüm sırası** (yukarıdan
aşağı yığın; `height() == None` olan esnek kart — bildirim merkezi — kalan alanı alır).

```toml
[panels.right]
enabled = true
size    = 380.0
widgets = ["session", "volume", "brightness", "network", "bluetooth", "power", "media", "notifications"]
```

- Ad kaynağı tek: `src/widgets/registry.rs` `WidgetKind` + `placement().panel`.
- Doğrulama (`config.rs` `filter_widgets`, bar ile aynı fonksiyon): bilinmeyen ad, panele konamayan
  tür (`clock`, `tray`…) ve aynı panelde tekrar → uyarıyla atılır; çalışma durmaz. Aynı kart farklı
  panellerde olabilir (her panel kendi örneğini kurar).
- Anahtar adı `widgets` kalır (görevde önerilen `cards` yerine): bar'la aynı ad, mevcut config'ler
  bozulmaz.

**Ayarlar sözleşmesi.** `nuxut-shell schema` çıktısındaki her `[[widget]]`
tablosunda `panel = true|false` zaten yazılıdır; panel içerik düzenleyicisi yalnızca
`panel = true` olanları "ekle" menüsünde sunar (sıra = `[[widget]]` sırası), listeyi
`[panels.<kenar>].widgets` dizisi olarak yazar (bar bölümlerindeki `bar.start` düzenleyicisiyle
aynı bileşen: ekle / sürükle-sırala / kaldır; aynı panelde tekrar yok). Kenarın kendisi
`panels.<kenar>.enabled` ve `panels.<kenar>.size` `[[setting]]` girdileriyle gelir. Yeni şema
alanı gerekmedi; `version` 1 kalır.

## 2. Hangi kenarlarda panel olabilir

Dört kenarın hepsinde (`left`, `right`, `top`, `bottom`); her kenarda en fazla bir panel. Sol panel
varsayılan kapalı (`enabled = false`, `widgets = []`); açmak için `[panels.left] enabled = true`
ve içerik listesi, tetik olarak bar düğmesi (`[bar.widget.panel-toggle] panel = "left"`), bar uç
eylemi (`left_click = "panel:left"`) ya da `nuxut-shell msg panel toggle left`.

- Bar'ın kenarındaki panel bar'ın iç kenarından büyür (bar solda → sol panel bar'dan sağa doğru).
  Çakışma yok: panel iç alanda, bar şeridi dışında kalır.
- Bildirim popup sütunu bar kenarının end köşesindedir; sabitlenmiş panellerin dışındaki alana
  yerleşir (aşağıda).

## 3. Sabitleme (pin)

### Durum makinesi (`src/panels.rs` `PanelState`, saf, birim testli)

- **Geçici yaprak** (en fazla bir): saat popout'u ya da sabitlenmemiş bir kenar paneli.
- **Sabitlenmiş paneller** (kenar başına en fazla bir, sabitleme sırasıyla): açık ve sabit.

| Olay | Geçici yaprak | Sabitlenmiş panel |
|---|---|---|
| Dışarı tıklama (çerçeve şeridi), odak kaybı (başka pencereye tıklama), Escape | kapanır | **kalır** |
| Sabit panelin boş yerine / eylemsiz yerine tıklama | kapanır (dışarı tıklama sayılır) | — |
| Başka yaprak açıldı | kapanır (yenisi geçici olur) | **kalır**, yenisiyle birlikte açık |
| Bar'dan menü açıldı (tepsi menüsü, bağlam menüsü; §5) | kapanır (tıklama anında) | **kalır**; menü onun dışına yerleşir |
| Aynı düğme / `msg panel toggle\|close <kenar>` | kapanır | kapanır (sabitleme biter) |
| `msg panel close-all`, `msg bar hide` | kapanır | kapanır |
| Pin düğmesi | sabitlenir | sabitleme kalkar, panel açık kalır ve geçici olur (başka geçici yaprak varsa o kapanır) |
| Tam ekran pencere | (komutla açılabilir) | geçici olarak gizlenir, tam ekran bitince geri gelir |

Escape sabitlenmiş paneli kapatmaz: sabitleme "yalnızca açık kapatma" demek; Escape metin
alanında/menüde de kullanılır, yanlışlıkla kapanmasın. (Kullanıcı kararı noktası.)

**Kalıcılık:** oturumluk ve açılışa bağlı — panel açıkça kapatılınca sabitleme düşer; bir sonraki
açılış sabitlenmemiş başlar. `shell.toml`'a yazılmaz (shell config'i asla yazmaz).

**Komut:** `nuxut-shell msg panel pin <kenar> [toggle|on|off]` (vars. `toggle`). `on` kapalı paneli
açar ve sabitler — bind ile "sabit aç" için.

### Klavye

Geçici yaprak ya da overlay açıkken eski politika (önce `Exclusive`, odak gelince `OnDemand`;
komutla açılan `Exclusive` kalır). Yalnızca sabitlenmiş panel(ler) açıkken klavye `OnDemand`:
kullanıcı başka pencerelerde yazar, panele tıklarsa (Wi-Fi şifresi) odak gelir; odak kaybı
sabitlenmiş paneli kapatmaz. Komutla açılıp sonra sabitlenen panel `Exclusive`'i bırakır.

### Odak kapma (Hyprland, yorum 7)

Sorun: başka uygulamanın açık menüsü (ör. Discord bağlam menüsü, `xdg_popup` grab'ı) açıkken
panel açılınca menü panelin üstünde kalıyordu. Çözüm `hyprland_focus_grab_v1` (framework
`cx.focus_grab`): geçici yaprak açıkken ana yüzey için grab tutulur; Hyprland grab başlarken
önceki grab'ı (menüyü) bitirir, odağı panele verir.

- Karar saf: `PanelState::wants_focus_grab` (yalnızca geçici yaprak) + `panels::focus_grab_step`
  (`Keep`/`Acquire`/`Release`/`Reacquire`), birim testli; uygulama `app/input.rs`
  `sync_focus_grab` — her input eşlemesinin sonunda.
- Escape, aynı düğme, komut, close-all: durum önce kapanır, aynı olay turunda grab **bırakılır**
  (`cleared` gelmez, kapanış çift tetiklenmez).
- Liste dışına (başka pencereye) tıklama → `focus_grab_cleared` → örtük kapatma (overlay +
  geçici yaprak; sabit panel kalır). Grab etkinken odak kaybı yaprağı kapatmaz; kapanışı
  `cleared` sürer.
- Sabitlenmiş panel grab tutmaz (pinlenince bırakılır); sabit + geçici açıkken grab yalnızca
  geçici içindir (tek ana yüzey: sabit panele tıklama grab'ı bitirmez).
- Klavye `Exclusive` → `OnDemand` inerken önce mod, sonra grab yeniden kurulur (noctalia sırası).
- Protokol yoksa (sway) hiçbir istek gitmez; odak kaybı yolu aynen sürer.

### Geometri: yer paylaşımı

Sabitlenmiş paneller iç alanı sırayla "oyar": her sabit panel, kendisinden önce sabitlenenlerin
kalan alanında yerleşir; geçici yaprak (kenar paneli ya da saat popout'u) tüm sabitlerin
dışında kalan alanda. Ör. sağ panel sabitken üst panel açılırsa üst panel sağ panelin bittiği
yere kadar uzanır (blob alanı ikisini fileto ile birleştirir); bar solda ve sol panel sabitken
saat popout'u sol panelin dış kenarından büyür. Sabitleme kalkınca/panel kapanınca diğer açık
paneller yeni alana yayla akar. Bildirim popup sütunu da sabit panellerin dışındaki alana
yerleşir; içinde `notifications` kartı olan bir panel açıkken popup'lar çöker (merkez orada).

### Pin düğmesi: köşe kuralı (`pin_corner`, saf, testli)

Düğme panelin çıktığı kenara yakın köşededir; diğer eksende bar'a yakın köşe:

- Sol/sağ panel: x = panelin kenarı (sağ panel → sağ). y = bar üstteyse üst, alttaysa alt; bar
  dikeyse üst.
- Üst/alt panel: y = panelin kenarı. x = bar soldaysa sol, sağdaysa sağ; bar yataysa sağ (end;
  bar'ın panel açan düğmeleri varsayılan olarak end bölümündedir).

**Yerleşim: içeriğin üstünde yüzer (yorum turu t100).** Düğme için satır ayrılmaz. Düğme
(`space(8)` = 32 px kare, `pin_rect`) panelin içerik dikdörtgeninin köşesinde, panel iç boşluğu
(`space(2)`) uygulanmadan durur ve içerikten sonra çizilir; içerik onun altından akar. İşaretçide
düğme panel içeriğinden önce gelir, ama isabet alanı yalnızca kendi 32 px karesidir.

Çakışmasızlık oluktan gelir: her panel widget'ı köşesinden içeriğine yatay/dikey boşluğunu
bildirir (`PanelWidget::corner_clearance`; varsayılan 0). Düğmenin altında kalan widget için
`pin_clearance_shift`: düğmenin widget'a yatay taşması bu boşluğa sığıyorsa itme yok; sığmıyorsa
yığın düğmenin bulunduğu üst/alt kenardan, düğmenin dikey taşması boşluğa inene kadar itilir (küçük iç
boşluk; yalnızca o kenarda, yalnızca gerektiğinde). Değerler (panel iç boşluğu 8 ile taşma 24):

| Widget | Boşluk (yatay, dikey) | İtme |
|---|---|---|
| Hızlı ayar kartları (`quick::Card`: ses, güç, ağ, Bluetooth) | sütun 16 + iç 8 = 24, 8 | yok (oluk) |
| Oturum kartı | 20, 20 | 4 px |
| Bildirim merkezi | 16, 16 | 8 px (kaydırılan listenin yuvarlak kart köşeleri düğmenin altından akabilir) |
| Parlaklık, medya (sütunsuz) | 8, 8 | 16 px |
| Bilinmeyen / yeni widget | 0, 0 | 24 px (widget düğmenin altına hiç girmez) |

Düğmenin altında hiçbir widget yoksa (ör. alt köşe, yığın panelin sonuna ulaşmıyor) itme yok.
Zemin diski çizilmez: ikon (≈20 px) oluğun içinde kalır, yalnızca hover katmanı kart yüzeyinin
köşesine biner.

Bileşen: framework `IconButton` (`IconButtonVariant::Standard`, `toggleable`, `IconName::Pin`;
seçiliyken dolu ikon, `primary` renk). Yeni framework bileşeni gerekmedi.

## 4. İçerik yerleşimi: hedef boyutta, her karede aynı (2026-10-13)

**Hata (kullanıcı):** bar saatine tıklayınca takvim popout'u "titreyerek" açılıyordu.

**Kök neden:** `Sheet::content_rect` içeriği kenar boyunca *görünen* (yayla büyüyen, taşan,
esneyen) dikdörtgenin boyutunda yerleştiriyordu. İçerik görünür olduğu açılış kesiminde (hedef
genişliğin ~%75'inden yay taşmasına, ~%101'e) takvim her karede başka genişlikte yeniden
diziliyordu: framework `CalendarGrid` hücresi `floor(genişlik / 7)` olduğu için ızgara piksel
adımlarıyla genişleyip daralıyor, yatay ortalama payı `(genişlik − 7·hücre) / 2` 0–3,5 px
arasında oynuyor, ızgara yüksekliği (`6·hücre`) 6 px'lik adımlarla değişip altındaki etkinlik
listesini sıçratıyordu; yay taşmasında (360 → 364 px) hücre bir piksel büyüyüp oturunca geri
küçülüyordu.

**Kanıt (izole sway, 60 kare/sn kayıt, popout bölgesi ölçümü):** yaprak genişliği
324→336→348→354→360→362→364→364→364→362→360 iken parlak içerik kutusu
257→272→281→287→293→294→298→298→296→294→293 px; ızgara 278 px/x=161'den taşmada 284 px/x=158'e
sıçrayıp geri dönüyordu. Kararlı sürüm (`c567983`, takvim kartı öncesi) aynı ölçümde aynı
yeniden yerleşimi gösterdi (250→267→281→293→296→294→293): regresyon değil, kenar sisteminden
(`0f9ce19`) beri vardı. Elenen adaylar: etkinlik cevabı yüksekliği değiştirmez (liste en çok
satır kadar yer ayırır, `preferred_height`), içerik ölçülmeden çizilmez (o tarihte ~%75'e kadar opaklık 0; bugünkü eşik §4.1),
saat/takvim kartı değişiklikleri (`25b2608`, `6a388a0`) yerleşim yoluna dokunmuyor.

**Karar:** içerik her karede hedef boyutta yerleşir; derinlikte bar'dan uzak kenara yapışık
(yaprakla birlikte iner — bilinçli hareket), kenar boyunca görünen dikdörtgenin ortasında
(popout ortadan büyür, esneme çapası ortada: içerik kenar boyunca yerinde durur). Görünen
dikdörtgen hedeften darken taşan kısım kırpılır. Düzeltme sonrası aynı ölçüm: içerik 293 px,
x=146, ızgara 278 px/x=161 tüm karelerde sabit (yaprak 364 px'e taşarken de). Bütün yapraklar
(popout, sistem izleyici, başlatıcı, kenar çekmeceleri) aynı `Sheet::content_rect`'i kullanır.
Bilinen sınır: açıkken yeniden hedeflenen (sabitleme ile yer paylaşımı) çekmecenin içeriği yeni
boyuta hemen geçer, yaprak yayla yetişene kadar kenarları kırpık görünebilir (eskiden her karede
yeniden yerleşiyordu). Regresyon testi: `sheet::tests::content_is_laid_out_at_target_size_on_every_frame`
(dört kenar, calm + lively, 60 kare/sn).

### 4.1 Yaprak hiçbir karede içeriksiz durmaz (2026-10-17)

**Hata (kare denetimi, izole sway kaydı):** açık bildirim yığınına yeni bildirim gelince yaprak
~130 ms (30 kare/sn'de 4 kare) boş koyu kutu olarak çiziliyor, görünen kartlar kaybolup
soluklaşarak geri geliyordu; her büyümede açılış geçişi baştan oynuyordu. Kapanıp yeniden
açılışta da (ve bütün yapraklarda: saat popout'u, başlatıcı, sistem izleyici, kenar
çekmeceleri) yaprak birkaç kare içeriksiz büyüyordu.

**Kök neden:** yüzey yeniden yaratılmıyor ya da boyutlanmıyordu (yapraklar çerçeve yüzeyindeki
blob'lar). `Sheet::content_alpha` içeriği görünen/hedef oranı kenar boyunca ~%75'i, derinlikte
~%80'i geçince açıyordu; üstüne `reveal` yayı (`motion.effects`, ~200 ms) açılışta 0'dan
başlıyordu. Açık yığına kart gelince hedef büyüyor, yaprak bir an hedefinin yarısı kadar kalıyor,
oran eşiğin altına düşüp içerik sıfırlanıyordu. Üst bar'da içerik uzak kenara yapıştığı için eski
kartlar yeni kartın boyu kadar yukarı sıçrayıp yaprakla geri iniyordu.

**Karar (tek kaynak `Sheet`, bütün yapraklar):** içerik yaprakla birlikte büyüyerek çıkar
(DESIGN.md: "şekiller birbirinden büyüyerek çıkar"); görünen dikdörtgene kırpılır ve uzak kenarla
iner. Opaklık büyüme ilerlemesine bağlı: tohumdan hedefe (derinlik 0'dan, kenar boyunca tohum
uzunluğundan) iki eksenin gerisinde kalanı %30'u geçince belirmeye başlar, %70'te tamdır
(yumuşak geçiş, `CONTENT_FADE_FROM/TO`). Kırpılarak gösterim tek başına seçilmedi: tohum anında
içerik yaprağın kenarına yapışık, kırpık ince bir metin şeridi olurdu; %30'a kadar yaprak yalnız
blob olarak büyür (30 kare/sn'de en çok 1 kare). `reveal` yalnız kapanış solmasıdır: tohumdan
açılışta 1'e atlar. İçerik bu açılışta bir kez tam göründüyse (`Sheet::is_revealed`) eşik tohumdan
yeni açılışa dek uygulanmaz: açıkken yeniden hedefleme (yeni bildirim kartı, sabitleme ile yer
paylaşımı) içeriği gizlemez, kapanırken yarıda geri açılan yaprakta içerik sönükleşmeden yayla
geri gelir. Bildirim yığınında mandal açıkken üst bar'da yığın kökeni bar tarafında kalır; yeni
kart yalnız kendi girişini (`NotificationStack` solması + kayması) oynatır, eskiler yığın yayıyla
kayar.

**Ölçü ile hedef aynı karede:** kart kurma, ölçme ve yığın dizimi `NotificationPopups::draw`'dan
`shapes`'e taşındı (Painter orada): yeni kartın ölçüsü ve yaprağın yeni hedefi aynı karede.
Eskiden yaprak bir kare geriden geliyordu.

**Doğrulama:** `sheet::tests::sheet_never_shows_empty_once_grown` (dört kenar, calm + lively, tohum
×0.5 ve ×1: tohumdan açılış, açıkken büyüme, kapanırken geri açılış, oturduktan sonra yeniden
açılış; büyüme %70'i geçince içerik tam, %35'ten sonra hiç sıfır değil, geri açılışta azalmaz),
`notification_popups::tests::growing_open_stack_never_draws_a_blank_frame` (düzeltmesiz sürümde
ilk büyüme karesinde opaklık 0 ile kırılır). Canlı: `tools/live/notifications/` (izole sway,
geçici HOME, özel D-Bus; 60 kare/sn kayıt, `frames.sh` kare dökümü) — büyüme, son kartın çekilip
yeniden açılması ve tohumdan açılışta popup sütununun gri sapması boş masaüstü seviyesinin altına
inmiyor, kareler gözle incelendi.

## 5. Tek geçici yüzey: bar menüleri ve sabit paneller (karar: 2026-10-13)

**Hata (kullanıcı):** sağ panel açıkken tepsiden bir öğeye sağ tıklayınca menü panelin üstüne
biniyordu. **Kök neden:** tepsi menüsü bar'ın overlay'i; sınırı (`WidgetCx::bounds`)
çerçevenin tüm iç alanıydı ve menü açılışı panel durumuna hiç dokunmuyordu — geçici panel açık
kalıyor, sabit panelin alanı menü için "boş" sayılıyordu. Popout'lar (saat, sistem izleyici)
yaprak oldukları için kurala zaten uyuyordu (geçiciyi değiştirir, sabitlerin dışındaki alanda
açılır).

**Kural:**
1. **Panel sabitlenmemişse:** bar'dan bir menü/popout açılınca açık geçici yaprak kapanır (kendi
   kapanış animasyonuyla, menünün ağacını beklemeden — tıklama anında). Tepsi: `OpenMenu` ve
   uygulamanın kendi bağlam menüsü (`ContextMenu`), `TrayCommand::opens_menu`; karar
   `PanelState::closed_by_bar_menu` (saf, testli), uygulama `App::act` (`Action::Tray`) →
   `Action::ClosePanel`. Popout'lar `PanelState::open` ile öncekini zaten kapatır.
2. **Panel sabitse:** menü panelin alanını dolu sayar. Ortak yerleşim alanı
   `panels::free_area` (iç alan − görünen sabit paneller; tam ekranda gizlenen sabit sayılmaz):
   popout'lar bu alanda yaprak olarak yerleşir, bar overlay'leri (tepsi menüsü, ipucu) bunu
   `WidgetCx::bounds` olarak alır. Menü tepsi öğesine bağlı kalır (yönü serbest alana göre,
   `menu_anchor`), framework yerleşimi (`Menu` → `grow::place`) sığmazsa yönü çevirir ve
   sınırın içine kaydırır; serbest alan menüden darsa menü daralır. Böylece panelle asla üst
   üste binmez. Sabitleme menü açıkken değişirse menü yeni sınıra taşınır (`Menu::set_bounds`).
   Framework talebi gerekmedi: dışlama, sınır dikdörtgeni olarak verilebiliyor.

Bildirim popup sütunu ve OSD bu kurala girmez (bar'dan açılmazlar; yerleri §3 "Geometri").
Testler: `panels::tests::bar_menu_closes_unpinned_panel_only`,
`free_area_excludes_every_pinned_panel`, `widgets::tray::tests::menu_opening_commands`,
`pinned_exclusion::*` (sağ/sol panel, dikey bar, alt bar + yön, dar çıkış). Canlı (izole sway,
sahte SNI): geçici sağ panel + sağ tık → panel kapanır, menü ikonun altında; sabit sağ panel +
sağ tık → menü bar'a bağlı, panelin iç kenarının solunda.
