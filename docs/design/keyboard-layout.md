# Klavye düzeni göstergesi (`keyboard_layout`)

Durum: uygulandı (2026-10-16). İstek: kullanıcı (widget kataloğu P1 #2, onay 2026-10-15). Küçük
ürün kararları burada.

## Davranış

- Bar'da etkin düzenin kısa adı ("TR", "US"); aynı düzen iki kez varsa varyantla ("US·INTL").
  Yatayda klavye ikonu + ad, dikeyde yalnız ad (kalınlığa sığar).
- **Yalnız 2+ düzen varken görünür**; tek düzenli makinede uzunluk 0 (varsayılan bar'da yer alsa
  da boşluk açmaz). Varsayılan bar: `end`'de `privacy` ve `magnifier`'dan sonra.
- Tık: ana klavyede sıradaki düzen (`switchxkblayout current next`). İyimser güncelleme yok:
  yeni ad Hyprland'in `activelayout` olayıyla gelir (kısayolla değiştirmeyle aynı yol).
- İpucu: etkin düzenin xkb adı ("Klavye düzeni: Turkish · tıklayınca sıradaki").
- Sakin hap (dolgusuz, `on_surface`): durum bildirimi, uyarı değil. Tekerlek geçişi yok
  (bar widget'larında tekerlek kanalı yok; kısayol ve tık yeterli).
- Ayarı yok; düzen listesi `[input] keyboard.layouts`'ta (Ayarlar › Klavye).

## Veri

- Kaynak Hyprland `j/devices`: ana klavye (`main: true`, yoksa ilk gerçek klavye; adında
  `virtual` geçen sanal klavyeler — ekran klavyesi — sayılmaz), `layout`/`variant` virgüllü
  listeleri, `active_layout_index`, `active_keymap`. Ayrıştırıcı saf ve sınırlı
  (`services::keyboard_layouts`: en çok 8 düzen, kodlar `[A-Za-z0-9_-]` ≤ 32, bozuk yanıt yok sayılır).
- Sorgu Hyprland servisinin thread'inde, yalnız bağlanınca, `activelayout` olayında ve ayar
  yeniden yüklemesinde (`kb_layout` değişmiş olabilir); değişmediyse olay yok. Boşta iş yok.
- Framework `Context::keyboard_layout` kullanılmadı: xkb grubu yalnız odaklı istemciye gider,
  bar odakta değilken bayat kalır.

## Güvenlik

Geçiş Hyprland'e komut olduğundan `SyncEnv::may_control` ile korunur: test örneği kullanıcının
oturumuna asla yazmaz (büyüteçle aynı kural; yalnız `NUXUT_TEST_HYPR_CONTROL=1` + `/run/user`
dışı çalışma dizinindeki iç içe/sahte Hyprland).

## Test

Birim: `keyboard_layouts::tests` (ana klavye seçimi, etiketler, tek düzen, bozuk girdi, sınır),
`widgets::keyboard_layout::tests` (görünürlük, ipucu, tık eylemi, dikey ölçü). Canlı: izole sway +
`examples/fake_hyprland` (`j/devices` `tr,us`; `switchxkblayout` → `activelayout`): bar'da "TR",
hapa sanal işaretçiyle tık → "US".
