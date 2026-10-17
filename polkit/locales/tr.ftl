## nuxut-polkit: yönetici yetkisi diyaloğu (Türkçe kaynak)

# Nerede: diyalog kartının başlığı. Anlam: bir işlem yönetici yetkisi istiyor.
# [max: 32]
polkit-title = Yetki gerekiyor
# Nerede: polkit boş mesaj gönderirse başlığın altındaki gövde metni.
polkit-default-message = Bu işlem için kimlik doğrulama gerekiyor.
# Nerede: mesajın altındaki küçük ayrıntı satırı (isteyen süreç bilinmiyorsa).
# Değişkenler: $action (polkit eylem kimliği, örnek: org.freedesktop.timedate1.set-time).
polkit-detail = Eylem: { $action }
# Nerede: aynı ayrıntı satırı, isteyen sürecin yürütülebilir dosya yolu biliniyorsa.
# Değişkenler: $app (doğrulanmış sistem yolu: root sahipli, yalnız root yazabilir; örnek: /usr/bin/timedatectl), $action (eylem kimliği).
polkit-detail-requester = Eylem: { $action } · İsteyen: { $app }
# Nerede: aynı ayrıntı satırı, yürütülebilir dosya yolu biliniyor ama güvenilir bir sistem
# konumunda değilse (ev dizini, /tmp, sahibi root değil, yazılabilir ya da olağan dışı
# karakter içeriyor). Anlam: uygulama sistem uygulaması gibi görünse de doğrulanmadı.
# Değişkenler: $app (yol; ev dizini ~ ile, örnek: ~/.local/bin/araç), $action (eylem kimliği).
polkit-detail-requester-unverified-path = Eylem: { $action } · İsteyen: { $app } (doğrulanmamış konum)
# Nerede: aynı ayrıntı satırı, yalnızca sürecin kendi bildirdiği ad biliniyorsa. Anlam: bu adı
# süreç kendisi seçebilir, doğruluğu denetlenmedi; kullanıcı buna güvenmemeli.
# Değişkenler: $app (süreç adı, örnek: udisksd), $action (eylem kimliği).
polkit-detail-requester-unverified = Eylem: { $action } · İsteyen: { $app } (doğrulanmamış ad)
# Nerede: parola alanının yer tutucusu (PAM istem metni yoksa).
# [max: 24]
polkit-password = Parola
# Nerede: diyalog düğmesi. Anlam: isteği reddet, diyaloğu kapat.
# [max: 16]
polkit-cancel = İptal
# Nerede: diyalog birincil düğmesi. Anlam: girilen parolayla kimliği doğrula.
# [max: 16]
polkit-authenticate = Doğrula
# Nerede: parola alanının altındaki hata satırı.
polkit-wrong-password = Parola yanlış. Yeniden deneyin.
# Nerede: parola gönderildikten sonra durum satırı.
polkit-verifying = Doğrulanıyor…
# Nerede: durum satırı, Caps Lock açıkken.
polkit-caps-lock = Caps Lock açık
