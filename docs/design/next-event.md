# Sıradaki etkinlik (`next_event`)

Durum: uygulandı (2026-10-16). İstek: kullanıcı (widget kataloğu P1 #5, onay 2026-10-15). Küçük
ürün kararları burada.

## Davranış

- Bar'da "Ekip toplantısı · 12 dk": `lead_minutes` (varsayılan 60, 5..=720) içinde başlayacak en
  yakın etkinlik. Yoksa süren etkinliklerden en son başlayanı: "Odak · bitiş 14:30". Hiçbiri
  yoksa gizli (uzunluk 0). Varsayılan bar: `center`'da saatten hemen sonra.
- Metin: < 1 dk "· şimdi", < 60 dk "· N dk", daha uzaksa "· 14:00". Başlık 24 karakterde "…" ile
  kısaltılır (tamamı ipucunda), adsız etkinlik "Adsız etkinlik".
- Tüm gün etkinlikleri ve iptal edilenler sayılmaz (bar'da sürekli duran bir "Bayram" gürültüdür).
- Başlamasına ≤ 5 dk kala hap `primary` dolguya geçer (caffeine/büyüteçle aynı "dikkat" dili);
  öncesinde sakin hap.
- Tık: saat popout'u (ay takvimi ve günün etkinlikleri). İpucu: başlık ve saat aralığı.

## Veri ve maliyet

- Kaynak `ShellModel::calendar` (`nuxut-calendard`, tek aralık sorgusu). Bar'da bu widget varsa
  `App` görünür takvim aralıklarına bugünün gece yarısından başlayan iki günü ekler
  (`next_event::near_range`): en uzun öncü süre (12 sa) günün her anından kapsanır, aralık
  günde bir değişir. Saat vuruşu (zaten dakikada bir) aralığı yeniden dener; aynıysa sorgu yok.
- Bilinen sınır: saat popout'unda çok uzak bir ay açılırsa birleşik aralık sorgu sınırını
  (`MAX_QUERY_SPAN`) aşar, gösterilen ay öncelikli olur; popout kapanınca yakın aralık geri gelir.
- Metin saat vuruşunda yenilenir (`BarWidget::set_time`); ek timer ya da uyanma yok.
- Takvim servisi başlamadan sorgu "gönderildi" sayılmaz (önceden ilk vuruş servisten önce
  gelirse aralık hiç gönderilmeyebilirdi).

## Test

Birim: `widgets::next_event::tests` (seçim: yaklaşan önce, sonra süren; tüm gün/iptal/eşik dışı
gizli; metinler; kısaltma; yakın aralık). Canlı: izole sway + yalıtılmış bus +
`examples/fake_calendar` (`FAKE_CALENDAR_SOON=12`): bar'da "Ekip toplantısı · 12 dk", daemon
günlüğünde iki günlük sorgu.
