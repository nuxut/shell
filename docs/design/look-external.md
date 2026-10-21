# Birleşik görünüm: yeniden yazım döngüsü ve başka aracın yönettiği dosyalar

Durum: uygulandı (1–3), tasarlandı (4: devralma, framework talebi bekliyor). 2026-10-16,
(görev). İlgili: ADR-0004 (birleşik görünüm), ADR-0001
(`nuxut-apply` sahiplik/snapshot), `src/services/look/{guard,writer,backend,gtk}.rs`.

## Olay

Kullanıcının gerçek makinesi (Arch + Hyprland, `hyprland.conf` end4/dots-hyprland ayarlarını
yüklüyor). "Uygulamalar da Nuxut gibi görünsün" açılınca Ayarlar "Görünüm bazı uygulamalara
aktarılamadı; ayrıntı kabuk günlüğünde" dedi. Günlükte saniyede bir aynı blok:
`look.mode externally_managed`, `look.icons applied`, `look.fonts applied`, canlı
`kde_fonts`/`kde_icons` sinyalleri ve her seferinde aynı uyarı.

## 1. Kök neden: dconf çift değeri bayt-eşit geri okunmuyordu (düzeltildi)

`nuxut-apply` dconf parçasını **metin** olarak karşılaştırır (`dconf read` çıktısı ile yazılacak
GVariant metni). Shell `text-scaling-factor`'ü `format!("{:.2}")` ile `1.00` yazıyordu; GLib
`g_variant_print` çiftleri `%.17g` + gerekirse `.0` biçiminde yazdırır. Yalıtılmış dconf'ta
ölçüldü (`dconf write` → `dconf read`, GLib 2.86):

| yazılan | okunan |
|---|---|
| `1.00` | `1.0` |
| `1.25` | `1.25` |
| `1.23` | `1.23` |
| `1.10` | `1.1000000000000001` |
| `0.90` | `0.90000000000000002` |

Sonuç: ölçek 1'de (varsayılan) look.fonts **hiçbir zaman `NoOp` olmuyordu**; her turda yeni
yazım + `kde_fonts` sinyali. Düzeltme: `gtk::gvariant_double` GLib biçimini birebir üretir
(0,01 adıma yuvarlanmış değer; ölçülen çıktılar testte). Diğer dconf değerleri (dizge, `true`,
tamsayı) zaten bayt-eşit geri okunuyor (aynı ölçüm).

**Framework talebi:** `nuxut-apply` dconf karşılaştırmasını GVariant anlamıyla yapsın (iki metni
de ayrıştırıp karşılaştırma ya da yazımdan sonra okunan metni `after` olarak saklama). Shell
düzeltmesi bu sınıf hatayı yalnızca kendi değerleri için kapatıyor.

## 2. Tetik: aynı plan yeniden uygulanmaz (düzeltildi)

Tasarım her tetikte (kurulu tema izleyicisi `ThemesChanged`, `framework.toml`/palet yeniden
yüklemesi) planı yeniden `nuxut-apply`'a verip `NoOp`'a güveniyordu. 1. maddedeki gibi tek bir
eşitsizlik, ya da Nuxut'un yazdığı bir anahtarı kendi biçiminde geri yazan başka bir araç
(ör. KDE `kded` `gtkconfig` modülü KDE sinyallerine tepki olarak GTK ayarlarını yeniden yazar),
her tetiği gerçek bir yazım + sinyal yapar.

Karar (`look::guard::ApplyGuard`, `nuxut-look` thread'i):

- Plan önceki uygulanan planla **aynıysa tur atlanır**: yazım, sinyal, log yok. Nuxut'un kendi
  yazımı ya da ilgisiz bir dosya olayı yeni tur doğurmaz.
- Dışarıdan geri alınan değer, girdi değişene ya da kullanıcı `nuxut-shell look apply` diyene
  kadar yeniden dayatılmaz (ADR-0004 §8-5: ezme yalnızca "bir sonraki uygulamada"; başka bir
  araçla saniyede bir kavga edilmez).
- Ayrıca test: Nuxut'un yazımları (imleç `~/.icons/default/index.theme` dahil) kurulu tema
  izleyicisini uyandırmaz (`own_writes_do_not_wake_theme_watcher`).

## 3. Sınırlı yineleme ve tek uyarı (düzeltildi)

- Bir ayar 60 sn içinde 10 kezden fazla **aynı içerikle** yeniden yazılırsa (başka araç sürekli
  geri alıyor ya da karşılaştırma eşitsizliği) 5 dk durdurulur; durdurma başlarken tek `WARN`.
  `msg look status` `last_apply.paused` listeler, `ok = false`.
- Yeni içerik sayılmaz (2026-10-16 düzeltmesi, §5): ilk sürüm her gerçek yazımı sayıyordu;
  duvar kâğıtları arasında hızlı gezinen kullanıcı look.mode'u 5 dk durduruyordu.
- Hata ve dış hedef uyarıları yalnızca sorun kümesi **değişince** bir kez loglanır; düzelince bir
  `INFO`.

## 4. `externally_managed`: karar, yeni davranış, devralma

### Kararın verilişi (`nuxut-apply`, değişmedi)

Dosya hedefi şu durumlarda `ExternallyManaged` sayılır ve üstüne yazılmaz: kırık symlink;
çözülen yol `/nix/store` ya da `/gnu/store`; dosya kullanıcıya ait değil; kullanıcının yazma biti
yok; sabit bağ (`nlink > 1`); yeni dosyanın en yakın var olan atası yazılamaz.

### Yeni davranış (uygulandı)

Önceden tek bir dışarıdan yönetilen dosya **bütün** look.mode'u (≈105 parça: kdeglobals renkleri,
GTK CSS, Qt şemaları) düşürüyordu. Şimdi `writer::ApplyBackend` o dosyanın parçalarını isteğe
çıkarıp kalan parçaları yine **tek işlemde** uygular (dosya sayısıyla sınırlı yeniden deneme;
yazılmayan parça snapshot'a girmez, "önceki hale dön" bölünmez). Rapor (`msg look status`
`last_apply.external`, sürüm 5): `{ setting, path: "~/…", reason }`, `reason` ∈
`symlink_broken | store | not_owned | read_only | hard_link | dir_not_writable | unknown`
(`writer::external_reason`, yalnızca `stat`). Kabuk günlüğüne dosya ve neden bir kez yazılır.

### Kullanıcıya gösterilecek (Ayarlar; settings deposu)

Genel "bazı uygulamalara aktarılamadı" yerine dosya başına:

- `read_only` / `hard_link` / `symlink_broken` ve bağlar artık devralınır (aşağıda); rapora
  `taken_over` olarak girer: "`~/.config/gtk-4.0/gtk.css` Nuxut'a devredildi; birleşik görünümü
  kapatmak önceki hâlini geri koyar."
- `not_owned` (düz dosya): kök gerektirmeden devralınır (framework bbf4a02, 2026-10-17): dosya
  okunabiliyorsa ve dizini yazılabilir, yapışkan değil ya da kullanıcınınsa yanına yazılan dosyanın
  `rename`'iyle; biçim `foreign:<izin>`, geri alma içerik ve izni geri koyar, sahip kullanıcı
  olur (kök olmadan sahiplik geri verilemez). Okunamayan ya da başkasının yapışkan dizinindeki
  dosya için "… başka bir kullanıcıya ait; Nuxut şimdilik değiştiremiyor." Kök komutu (`chown`)
  önerilmez (kullanıcı kararı 2026-10-17).
- `store`: "… Nix/Guix tarafından yönetiliyor; oradan değiştirin." (devralma yok).

### Devralma — varsayılan (uygulandı, 2026-10-16)

İlk tasarım devralmayı kullanıcının dosya başına onayına bağlıyordu ("Nuxut yönetsin" düğmesi).
Kullanıcı ilkesi (2026-10-16) bunu değiştirdi: **Nuxut yapabiliyorsa varsayılan Nuxut** — başka
aracın ayarı onay beklemeden, yedekli ve tek adımda geri alınabilir biçimde devralınır.

1. `nuxut-apply` `Change::take_over(yol)` (framework, değişmez 14): dosyanın Nuxut öncesi biçimi
   (`symlink:<metin>`, `mode:<sekizli>`, `hardlink`) işlemin snapshot'ına `file_form` parçası
   olarak girer; yol Nuxut'un düz dosyasına çevrilir (symlink → hedef içeriğinin kopyası, salt
   okunur → `u+w`, sabit bağ → bağı koparan kopya) ve parçalar yazılır. Tek işlem, tek snapshot.
2. Shell (`writer::ApplyBackend`) `ExternallyManaged { reason }` alınca neden devralınabiliyorsa
   o yolu devralma listesine ekleyip yeniden dener; devralınamıyorsa (yukarıdaki "yeni davranış")
   dosyayı dışarıda bırakır.
3. "Önceki hale dön" / birleşik görünümü kapatma özgün dosyayı (symlink, izin, bağ) geri koyar.
4. `store` ve başka kullanıcının düz dosyası devralınmaz; root'un dosyasına **bağ** ise bağın
   kendisi kullanıcınındır, devralınır (hedefe dokunulmaz).
   **Karar yalnız bağa bakar (2026-10-17):** `~/.config` içindeki symlink, hedefinin sahibi,
   izni ya da konumu ne olursa olsun devralınır (`nuxut_apply::takeable_link`; yalnız nix/guix
   deposu hariç); Nuxut hedefe asla yazmaz. Gerçek makinede `gtk-4.0/gtk.css` →
   `/usr/share/themes/Dracula/gtk-4.0/gtk.css` hedefinin sahibi elle kullanıcı yapılmıştı:
   eski kural hedefi "kullanıcının yazılabilir dosyası" sayıp içinden yazmaya çalışıyor, root'un
   dizininde `rename` başarısız oluyor, neden `unknown` raporlanıp bağ hiç devralınmıyordu
   (Nautilus Nuxut renklerini almıyordu). Artık bağlar plan aşamasında devralınır; var olan
   dosyanın dizini yazılamıyorsa neden `dir_not_writable`. Kullanıcının dotfiles deposuna bağ da
   aynı kuralla devralınır (birleşik görünümü kapatmak bağı geri kurar).
5. Kalıcı kayıt `nuxut-apply` snapshot'ıdır (`shell.toml`'a yazılmaz). Ayarlar için
   `msg look status` `last_apply.taken_over`. Ayrıntı: docs/design/look-gtk.md "Devralma".

## 5. Hızlı seçimler: son seçim kazanır (2026-10-16, look)

Kullanıcı: "duvar kâğıdı seçtim, renkler geçti; sonra nötr seçtim; KDE geç geçti, sonra nötre
daha sonra geçti". Kabuğun kendi renkleri anında değişiyordu; gecikme `nuxut-look` thread'indeydi.

Nedenler:

1. **Kuyruk:** tur (≈6 ayar × `nuxut-apply` işlemi + dconf alt süreçleri + KDE sinyalleri)
   sürerken gelen yeni seçim turun bitmesini, ardından 400 ms birleştirmeyi bekliyordu; ara durum
   (eski seçim) KDE'ye tam uygulanıp sinyallenmeden yenisine geçilmiyordu.
2. **Bekçi:** §3'teki ilk sınır her gerçek yazımı saydığından hızlı gezinme look.mode'u 5 dk
   durduruyordu; süre dolunca da tetik gelmediği için renk ancak bir sonraki değişimde geçiyordu.

Karar:

- Tur her ayardan önce kuyruğa bakar (bloklamadan). Yeni istek varsa kalan ayarlar ve **bütün**
  canlı eylemler atlanır, en yeni istekle hemen yeniden başlanır (`backend::apply_with`,
  `app/look.rs` `newer_request`). Yarıda kalan turun yazdığı ayarlar `carried` olarak taşınır:
  KDE/ikon/imleç sinyalleri yalnızca tamamlanan son turda, tek sefer ve son durumla gider.
- Yarıda kalan tur bekçinin "son uygulanan plan"ını siler (`ApplyGuard::forget`): A → B (yarıda)
  → A dizisinde A yeniden uygulanır.
- Birleştirme süresi 400 → 120 ms (yalnızca aynı seçimin palet + `framework.toml` yazımlarını
  birleştirir; daha uzun istekler zaten yarıda kesilir).
- Bekçi yalnızca aynı içeriğin yeniden yazımını sayar (§3).
- Gecikme ölçümü: `INFO birleşik görünüm uygulandı … ms=` (turun okuma + yazım + sinyal süresi);
  yarıda bırakılan turlar `DEBUG`.

Ölçüm (`cargo test latency_of_a_colour_change -- --ignored --nocapture`; geçici kök tmpfs'te,
test örneği: dconf ve sinyal yok): ilk tur (6 ayar) ≈ 45 ms, renk değişimi (yalnız look.mode)
≈ 20 ms. dconf alt süreci yalıtılmış oturumda okuma ≈ 11 ms, yazma ≈ 9 ms (look.mode'da 2 anahtar).
Yani tur ≈ 0,1 sn; kullanıcının gördüğü saniyeler–dakikalar 400 ms birleştirme + kuyruk + en çok
da bekçinin 5 dk'lık durdurmasıydı.

## Kullanıcının deneyeceği

Yeni sürümle: günlükte saniyede bir blok kalmaz (ilk turdan sonra tetikler sessizce atlanır).
Dışarıdan yönetilen dosya varsa günlükte bir kez `dosya başka bir araç tarafından yönetiliyor`
satırı `path` ve `reason` ile görünür; `nuxut-shell msg look status` `last_apply.external` aynı
bilgiyi verir. Renkler o dosya dışındaki tüm hedeflere uygulanır.
