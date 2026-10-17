# Workspace göstergesi — sayfalı, durumlu, kaydırmalı

Kaynak: kullanıcı yorumu 9 (2026-10-10). Kod: `src/widgets/workspaces.rs` (görünüm, kaydırma),
`src/model.rs` (`workspace_page` — saf sayfa mantığı), `app_glyphs.rs` framework'te (`nuxut-ui`, sınıf → glif),
`src/services/hyprland.rs` (workspace başına temsilci pencere sınıfı).

## Şema — `[bar.widget.workspaces]`

```toml
count  = 10        # sayfa başına nokta, 1..=30 (dışı uyarıyla sınırlanır)
style  = "icons"   # "icons" | "numbers" | "dots"
scroll = "all"     # "all" (sıradaki id, boş dahil) | "occupied" (yalnızca dolu/açık) | "off"
```

- `count` anlamı değişti: eskiden "`show_empty` iken 1..=count", şimdi **sayfa boyu**. Anahtar adı
  korunur (Ayarlar kataloğu ve mevcut dosyalar kırılmaz); üst sınır 50 → 30.
- `show_empty` kalktı: her zaman `count` nokta görünür. Eski dosyada kalırsa bilinmeyen anahtar
  uyarısıyla yok sayılır.

## Sayfa mantığı (saf, birim testli)

- Sayfa `p` (0'dan) = id'ler `p·N+1 ..= p·N+N`. Monitörün sayfası = **o monitörün aktif
  workspace'inin** sayfası: `(id − 1) / N`.
- Aktif workspace numarasızsa (adlı workspace, bilinmiyor) sayfa değişmez: widget son sayfayı
  hatırlar (`fallback_page`), hap gizlenir. Special workspace monitörün `activeWorkspace`'ini
  değiştirmez; gösterge normal sayfada kalır.
- **Kayda değer** workspace: penceresi olan ya da herhangi bir monitörde görünen (aktif).
  Kalıcı (persistent) ama boş ve görünmeyen workspace kayda değer değildir.
- `<` (dikey bar'da yukarı): sayfa başından küçük, ≥ 1 kayda değer id varsa; hedefi bunların
  **en büyüğü** (bir önceki dolu workspace). `>` (aşağı): sayfa sonundan büyük kayda değer id;
  hedefi bunların **en küçüğü**. Göstergeye tıklamak o workspace'e geçer.
- Göstergelerin yeri **her zaman ayrılır** (görünmezken de): noktalar hiç kaymaz. Ölçü:
  `ok + N·öğe + (N−1)·boşluk + ok + ayrılmış yuva` (aralarda boşluk).

## Durumlar ve görsel (DESIGN.md: tonal, sabit renk yok)

| Durum | Zemin | İçerik |
|---|---|---|
| Bu monitörde aktif | `primary` hap (yayla kayar) | `on_primary` |
| Başka monitörde görünür | tonal disk: `primary` %18 | `primary` |
| Dolu (pencereli) | — | `on_surface`; ikon/numara ya da büyük nokta |
| Boş | — | küçük nokta, `on_surface_variant` %50 |
| Hover / basılı | state layer (`Interaction`, %8/%12) | — |

- **Stiller:** `icons` — dolu workspace'te temsilci pencerenin uygulama glifi; glif bulunamazsa
  numara (geri düşüş), boşta nokta. `numbers` — her yuvada numara (boş soluk). `dots` — yalnızca
  nokta (dolu büyük, boş küçük).
- **Temsilci pencere:** workspace'teki en büyük alanlı eşlenmiş, gizli olmayan pencere; eşitlikte
  en son odaklanan (`focusHistoryID` küçük). Odak değiştikçe ikon zıplamaz.
- **Glif:** Nerd Font (`nf-md-*`, `nf-linux-*`, `nf-dev-*`, `nf-custom-*`, `nf-fa-*`) kod noktası;
  tablo framework'te (`app_glyphs.txt`), `glyphnames.json` 3.5.1'den doğrulanmış. Eşleme sınıf küçük harfle:
  tam ad → noktalı adın son parçası (`org.gnome.Nautilus` → `nautilus`) → `-bin`/`-git` gibi
  ekler atılmış ad → `steam_app_*` (oyun) → `libreoffice-*`, `jetbrains-*` önekleri.
  Font: `Symbols Nerd Font` (Arch `ttf-nerd-fonts-symbols`), yoksa temanın `mono` ailesi (varsayılan
  `JetBrainsMono Nerd Font`); ikisi de kurulu değilse `icons` stili numaraya düşer.
- Sayfa değişince hap kaymaz, yeni yerine atlar (10'dan 11'e geçişte bar boyu kayan hap yanlış
  yön söyler). Sayfa içi geçişte mevcut yay davranışı korunur.
- Oklar framework `IconView` (`chevron_left/right/up/down`, `IconSmall`), `on_surface_variant`;
  görünürlük `motion.effects` ile solar.

## Kaydırma

- Tekerlek: her çentik (`steps`) bir adım. Dokunmatik yüzey (yalnızca `delta`): piksel birikir,
  her **48 px** bir adım; 400 ms sessizlik ya da yön değişimi birikimi sıfırlar. Aşağı/sağ = sonraki.
- Hedef shell'de hesaplanır (saf): `all` → `aktif ± adım` (alt sınır 1); `occupied` → sıradaki
  kayda değer id (yoksa hareket yok). Komut `dispatch workspace <id>` (`WorkspaceTarget::Id`).
- Hızlı kaydırmada Hyprland olayı gelmeden ikinci adım bayat aktif id'den hesaplanmasın diye
  widget son gönderdiği hedefi 600 ms "bekleyen" tutar; sonraki adım ondan devam eder.
- **Bar uç eylemleriyle öncelik** (`[bar.edge_actions]`, `EdgeBinding::route`): workspaces uç
  widget'ıysa ve `scroll`/`left_click`/`right_click` `widget` dışında bir eyleme atanmışsa uç eylemi
  kazanır; `widget` (varsayılan) ise olay widget'a gider. Uç eylemi `scroll = "workspace"` da aynı
  yolu kullanır (`WorkspaceScroller` + `scroll_target`; `scroll = "off"` iken `all`); `e±n` göreli
  dispatch kalktı.
- Kaydırma o anki fare altındaki monitörün bar'ında olur; hedef workspace başka monitördeyse
  Hyprland'in kendi kuralı geçerlidir (tıklamayla aynı).

## Kenar durumları

- **Adlı workspace** (`workspace name:foo`, negatif id ≤ −1337): sayfalara girmez, ok
  hesaplarına katılmaz; aktifken sayfa hapı gizlenir, sayfa son numaralı sayfada kalır; ayrılmış yuvada gösterilir (aşağıda).
- **Negatif id / special:** `special:*` adlı ya da −99..=−2 id'li workspace special'dır; asla
  gösterilmez. (Eski `is_special = id < 0` adlı workspace'leri de special sayıyordu; düzeltildi.)
- **Çok monitör:** id'ler küreseldir; her bar kendi monitörünün sayfasını gösterir, başka
  monitörün görünür workspace'i tonal diskle işaretlenir. Monitör bilinmiyorsa sayfa 0.
- **Büyük id** (ör. 9999): sayfa hesabı taşmaz (`i64`), `count` ≤ 30.
- **Dikey bar:** aynı sıra y ekseninde; oklar yukarı/aşağı.

## Ayrılmış yuva: adlı ve special workspace

Sağdaki okun dışında bir öğe boyu yuva **her zaman ayrılıdır** (noktaları ve okları itmez):

- **Adlı workspace bu monitörde aktifken:** primary hap; içinde temsilci pencerenin uygulama
  glifi, yoksa adın büyük baş harfi (`on_primary`). Sayfa noktalarına girmez.
- **Special workspace bu monitörde açıkken:** sakin işaret — tek başına tonal disk + küçük
  `primary` nokta; adlı hap da varsa hapın köşesinde `surface` halkalı küçük rozet.
- Görünürlük `motion.effects` ile solar. Ad (ve `Özel çalışma alanı: <ad>`) ipucu balonunda:
  `BarWidget::tooltip` yalnızca bu yuvanın üstünde metin döndürür; balonu framework `Tooltip`'i
  `Screen` tarafında çizer.
- Tıklama yok (adlı workspace'e geçiş için serbest metin dispatch'i yok; zaten aktif).

## Kararlar (2026-10-10)

1. Varsayılan `style = "icons"` (dolu = uygulama glifi, boş = nokta, glif yoksa numara); `dots`
   ve `numbers` seçenek.
2. Adlı workspace'ler sayfa noktalarına girmez; aktifken ayrılmış yuvada ikonla, ad ipucunda.
3. Special workspace açıkken ayrılmış yuvada küçük, sakin işaret.
4. `scroll` varsayılanı `all`.
