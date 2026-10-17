# Bar uç eylemleri (yorum 5)

Bar'ın **en baştaki** (yatay: en sol, dikey: en üst) ve **en sondaki** (yatay: en sağ, dikey:
en alt) widget'ına sol tık, sağ tık ve kaydırma için özel eylem atanabilir. Örnek senaryo:
en sağda saat; kaydırma = ses, fare üzerindeyken saatin dış yanında ses ikonu belirir (saat
itilmez); en solda kaydırma = parlaklık; sol tık sağ paneli açar; sağ tık atanmamış.

## Şema

```toml
# Bunlar varsayılanlardır (kullanıcının örneği; karar).
[bar.edge_actions]
scroll_step = 5              # bir tekerlek çentiği = %5 (ses/mikrofon/parlaklık), 1..=25

[bar.edge_actions.start]     # yatay: en sol widget, dikey: en üst
left_click  = "widget"
right_click = "widget"
scroll      = "brightness"

[bar.edge_actions.end]       # yatay: en sağ widget, dikey: en alt
left_click  = "panel:right"
right_click = "widget"       # atanmamış: widget'ın kendi sağ tıkı kalır
scroll      = "volume"
```

Eylem kümesi (dizeler; geçersiz değer uyarıyla `"widget"`e döner):

| Değer | sol/sağ tık | kaydırma |
|---|---|---|
| `widget` | widget'ın kendi davranışı | widget'ın kendi davranışı (tepsi öğesi vb.) |
| `none` | hiçbir şey (widget'ınki de bastırılır) | hiçbir şey |
| `panel:left\|right\|top\|bottom` | o kenar çekmecesini aç/kapat | — |
| `panel:clock` | takvim popout'u | — |
| `panel:launcher` | uygulama başlatıcısı (`launcher.enabled = false` iken etkisiz) | — |
| `volume` | çıkışı sessize al / aç | çıkış sesi ± `scroll_step` |
| `mic` | mikrofonu sessize al / aç | mikrofon seviyesi ± `scroll_step` |
| `brightness` | — | bar'ın monitöründeki ekran ± `scroll_step` |
| `workspace` | — | workspace widget kaydırmasıyla aynı yol ve ölçüt (`[bar.widget.workspaces] scroll`: `all` sıradaki id, `occupied` yalnızca dolu/görünür; `off` iken `all`): yukarı = önceki, aşağı = sonraki (bkz. `docs/design/workspaces.md`) |

Tıkta anlamsız olan (`brightness`, `workspace`) ve kaydırmada anlamsız olan (`panel:*`) değerler
uyarıyla reddedilir; şemada her anahtar yalnızca geçerli seçeneklerini listeler.

**Varsayılanlar** yukarıdaki örnektir. Kısmi tablo yalnızca yazılan anahtarı değiştirir
(`[bar.edge_actions.end] scroll = "mic"` sol tıkı `panel:right` bırakır); geçersiz değer uyarıyla
o ucun varsayılanına döner. Sağ tık "atanmamış" = `widget` (ör. tepsi en uçtaysa kendi bağlam
menüsü kalır); `none` widget'ın davranışını da bastırır.

**Parlaklık yeteneği:** bar'ın monitöründe parlaklığı ayarlanabilen ekran yoksa (arka ışık/DDC
yok) `brightness` eylemi yok sayılır — kaydırma uç widget'a kalır (ör. workspace kaydırması) — ve
ikon yuvası ayrılmaz. Yuva servis durumuna göre dinamiktir: yalnızca yetenek değişince (ekran
takıldı/çıktı) eklenir/kalkar, hover'da asla.

## İpucu balonu

Bar widget'ları ortak bir framework `Tooltip` kullanır (`Screen` başına tek): fare hareketinde
`BarWidget::tooltip` metin verirse `hover(metin, widget dikdörtgeni, MenuSide::away_from(bar
kenarı), iç alan)`, yoksa `unhover`. Gecikme bileşende; beklerken kare yok — `hover`'ın ya da
çizimdeki `Ui::wake_deadline`'ın anı App'in tek uyanma timer'ına (`app/wake.rs`) gider, timer
yalnızca kare ister. Etkin pencere başlığı `max_chars` (grafem) ya da `max_width` (framework tek
satır "…") ile kesildiyse tam başlığı verir.

### Keyfi komut çalıştırma — eklenmedi

`exec:<komut>` gibi bir eylem bilinçli olarak yok. Gerekçe: `shell.toml` Ayarlar uygulaması ve
içe aktarılan tema/dotfile paketleri tarafından da yazılır; komut alanı, config'i yazabilen her
şeyi oturumda kalıcı kod çalıştırma yetkisine yükseltir (fare tıkı tetikleyici olur). Hyprland
zaten `bind`/`exec` sunuyor; komut isteyen kullanıcı Hyprland bind'i + `nuxut-shell msg` kullanır.
İleride gerekirse: ayrı, Ayarlar'ın yazmadığı bir dosyada (`~/.config/nuxut/actions.toml`),
`argv` dizisi (kabuk yok), mutlak yol, ortam temizlenmiş, `NUXUT_SHELL_INSTANCE` sızmadan,
zaman aşımı ve eşzamanlılık sınırıyla; varsayılan kapalı. Bu not o tasarımın ön koşuludur.

## Yerleşim: ayrılmış ipucu yuvası

Kaydırma eylemi ikonlu bir sistem eylemiyse (`volume`, `mic`, `brightness`, `workspace`) uç
widget'ın **dış yanında** bir ipucu yuvası baştan ayrılır: uzunluk = öğe boyu (`WidgetCx::item`)
+ `bar.spacing`. Yuva bölümün toplam uzunluğuna eklenir; `bar_slots` onu sıradan bir öğe gibi
görür (start bölümünde başa, end bölümünde sona). Böylece ikon belirince/kaybolunca hiçbir
widget yer değiştirmez ve orta bölümün yerleşimi değişmez. Yuva yalnızca o bölümde görünür bir
widget varken ayrılır (boş bölümde üzerine gelinecek widget yok).

Eylem bölgesi = uç widget'ın dikdörtgeni ∪ ipucu yuvası (bar kalınlığı boyunca). İkon yuvası
üzerindeki tık/kaydırma da uç eylemidir.

## İkonun belirmesi (DESIGN.md hareket dili)

İşaretçi eylem bölgesine girince ikon widget'ın yanından **büyüyerek** çıkar: `motion.spatial`
yayıyla 0→1 ilerleme; ikon widget'a doğru `space(2)` kaymış konumdan yerine oturur, opaklık
aynı ilerlemeyle gelir (ayrı doğrusal solma yok). Çıkınca yay tersine döner; yay kesilebilir
(hızlı gir-çık zıplamaz). `motion.enabled = false` → anında. Renk `on_surface_variant`
(ikincil öğe); ikon temanın ikon setinden (`IconView`, ADR-0006) ve canlı durumu gösterir
(ses seviyesi/sessiz, mikrofon kapalı, parlaklık düzeyi).

## Dikey bar

`start`/`end` bar ekseni boyunca: dikey bar'da en üst/en alt widget. Yuva y ekseninde öğe boyu
kadar; ikon widget'ın üstünde (start) ya da altında (end) belirir ve dikey kayar.

## Uç widget'ın kendi davranışıyla çakışma

Kural: atanmış (`widget` olmayan) eylem o girdide **önceliklidir**, widget o girdiyi görmez;
`widget` ise girdi değişmeden widget'a gider. Örnek: en sağda saat + `left_click = "panel:right"`
→ tık takvimi değil sağ paneli açar; `left_click = "widget"` → takvim. Tepsi en uçtaysa ve
`scroll = "volume"` ise tepsi öğelerine kaydırma gitmez. Tek istisna: açık overlay (tepsi
menüsü) varken olaylar önce ona gider (menü dışına tık menüyü kapatır), uç eylemi çalışmaz.

## Kaydırma birikimi

`PointerKind::Scroll { delta, steps }`: tekerlekte `steps` (yüksek çözünürlüklü tekerlekte
kesirli), dokunmatik yüzeyde yalnızca `delta` (mantıksal px; 40 px = bir çentik). Değerler
birikir, tam çentikler uygulanır, kesir kalır; yön değişince ya da 400 ms boşta kalınca birikim
sıfırlanır; `ScrollEnd` (parmak kalktı) kesri atar. Olay başına en fazla 10 çentik. Yukarı
kaydırma artırır. Yatay baskın kaydırma yok sayılır (diyagonal dokunmatik gürültüsü).

## Uygulama

- Ayrıştırma/doğrulama/şema/birikim: `src/bar_edge.rs` (saf, birim testli).
- `Screen::geometry` yuvayı ayırır, `BarGeometry::edges` bölgeleri taşır; `Screen::pointer`
  uç eylemini overlay'lerden sonra, bar isabet testinden önce dener; ikon çizimi bar
  içeriğinden sonra, bar şeridine kırpılı.
- `Action::BarEdge(EdgeCommand)` → `app/system.rs` `bar_edge_command`: `apply_volume`,
  `apply_brightness` (bar'ın monitörü), mikrofon `SystemCommand::Volume`/`SetMute`, workspace
  kaydırması `scroll_workspace` (widget ile ortak `WorkspaceScroller`, `dispatch workspace <id>`). OSD servis olayından gelir (`OsdTracker`); OSD monitörü bar'ın monitörü olarak istenir.
