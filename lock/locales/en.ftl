## Kilit ekranı: giriş ve durum

# Nerede: Kilit ekranı, şifre alanının yer tutucusu, alan boşken (lock/src/view.rs). Anlam: alanın ne için kullanıldığını hatlatan ipucu metni.
shell-lock-password-placeholder = Password

# Nerede: Kilit ekranı, şifre alanının yer tutucusu; parmak izi okunurken, eşleşmediğinde veya yeniden denenirken (lock/src/view.rs). Anlam: alan hem şifreyi hem parmak izini kabul ediyor.
shell-lock-password-placeholder-fingerprint = Password or fingerprint

# Nerede: Kilit ekranı, şifre alanının altındaki durum satırı; PAM doğrulaması sürerken (lock/src/view.rs). Anlam: giriş bilgileri kontrol ediliyor.
shell-lock-status-verifying = Verifying…

# Nerede: Kilit ekranı, şifre alanının altındaki hata satırı; PAM erişimi reddettiğinde ve kendi mesajını vermediğinde (lock/src/view.rs). Anlam: girilen şifre yanlış.
shell-lock-error-wrong-password = Wrong password

# Nerede: Kilit ekranı, şifre alanının altındaki hata satırı; faillock gibi bir modül deneme sınırına ulaşıldığını bildirdiğinde (lock/src/view.rs). Anlam: çok fazla deneme yapıldı, şimdilik giriş yapılamaz.
shell-lock-error-max-tries = Too many attempts

# Nerede: Kilit ekranı, şifre alanının altındaki hata satırı; kimlik doğrulama servisine (PAM) ulaşılamadığında (lock/src/view.rs). Anlam: doğrulama servisi yanıt vermiyor; sorun kullanıcının şifresinde değil.
shell-lock-error-unavailable = Authentication service unavailable

# Nerede: Kilit ekranı, şifre alanının altındaki hata satırı; PAM ek bir bilgi istediğinde, örneğin doğrulama kodu veya parola yenileme (lock/src/view.rs). Anlam: bu doğrulama adımı kilit ekranında desteklenmiyor, giriş için başka yol gerekir.
shell-lock-error-unsupported = This authentication step is not supported

# Nerede: Kilit ekranı, şifre alanının altındaki hata satırı; nedeni belirsiz bir doğrulama hatasında (lock/src/view.rs). Anlam: doğrulama başarısız oldu, neden bilinmiyor.
shell-lock-error-generic = Authentication error

# Nerede: Kilit ekranı, şifre alanının altındaki durum satırı; çok fazla denemeden sonra bekleme süresi geri sayılırken (lock/src/view.rs). Anlam: kalan bekleme süresi; süre bitince yeniden denenebilir.
# Değişkenler: $count (kalan bekleme süresi, saniye).
shell-lock-backoff =
    { $count ->
        [one] Try again in { $count } second
       *[other] Try again in { $count } seconds
    }

# Nerede: Kilit ekranı, şifre alanının altındaki uyarı satırı; Caps Lock açıkken (lock/src/view.rs). Anlam: büyük harf kilidi açık; şifre yanlış yazılıyor olabilir.
shell-lock-caps-lock = Caps Lock is on

# Nerede: Kilit ekranı, şifre alanının altındaki durum satırı; parmak izi okundu ama kayıtlı izlerle eşleşmediğinde (lock/src/view.rs). Anlam: bu parmak izi tanınmadı; kalan deneme sayısını gösterir.
# Değişkenler: $count (kalan parmak izi denemesi).
shell-lock-fingerprint-no-match =
    { $count ->
        [one] Fingerprint did not match ({ $count } try left)
       *[other] Fingerprint did not match ({ $count } tries left)
    }

# Nerede: Kilit ekranı, şifre alanının altındaki durum satırı; okuyucu parmağın yeniden okutulmasını istediğinde (lock/src/view.rs). Anlam: parmak izi tam alınamadı, yeniden denenecek.
shell-lock-fingerprint-retry-scan = Scan your finger again

# Nerede: Kilit ekranı, şifre alanının altındaki durum satırı; parmak hareketi çok hızlı veya çok kısa olduğunda (lock/src/view.rs). Anlam: tarama tamamlanamadı; daha yavaş kaydırmak gerek.
shell-lock-fingerprint-retry-too-short = Slide your finger more slowly

# Nerede: Kilit ekranı, şifre alanının altındaki durum satırı; parmak okuyucunun ortasında değilse (lock/src/view.rs). Anlam: parmağı okuyucunun ortasına yerleştirmek gerek.
shell-lock-fingerprint-retry-not-centered = Center your finger

# Nerede: Kilit ekranı, şifre alanının altındaki durum satırı; parmağın kaldırılıp yeniden konulması istendiğinde (lock/src/view.rs). Anlam: okuma yarım kaldı; parmağı kaldırıp tekrar okutmak gerek.
shell-lock-fingerprint-retry-remove = Lift your finger and try again

# Nerede: Kilit ekranı, şifre alanının altındaki durum satırı; parmak izi denemeleri tükendiğinde (lock/src/view.rs). Anlam: parmak iziyle giriş şimdilik kapandı; şifreyle devam edilecek.
shell-lock-fingerprint-exhausted = Fingerprint disabled; enter your password

# Nerede: Ekran okuyucu (yalnız Orca okur): kilit ekranı yüzeyinin adı. Anlam: oturum kilitli.
shell-lock-a11y-title = Lock screen

# Nerede: Ekran okuyucu: kilit ekranındaki şifre alanının adı (değer ve uzunluk okunmaz). Anlam: kilidi açmak için şifre.
shell-lock-a11y-password = Password

# Nerede: Ekran okuyucu: kilit ekranı şifre alanının durumu, alan boşken. Anlam: henüz bir şey yazılmadı.
shell-lock-a11y-password-empty = Empty

# Nerede: Ekran okuyucu: kilit ekranı şifre alanının durumu, bir şey yazılmışken (uzunluk söylenmez). Anlam: alana bir şey yazıldı.
shell-lock-a11y-password-filled = Filled

# Nerede: Ekran okuyucu: kilit ekranında art arda yanlış denemeden sonra bekleme (saniye söylenmez). Anlam: kısa bir süre sonra yeniden denenebilir.
shell-lock-a11y-wait = Wait a moment, then try again

# Nerede: Ekran okuyucu: kilit ekranında parmak izi eşleşmediğinde (kalan sayı söylenmez). Anlam: okutulan parmak izi tanınmadı.
shell-lock-a11y-fingerprint-no-match = Fingerprint not recognized
