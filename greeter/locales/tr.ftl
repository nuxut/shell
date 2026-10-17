## Giriş ekranı

# Nerede: Giriş ekranı, istem alanının yer tutucusu; PAM standart şifre istemi gönderdiğinde (greeter/src/view.rs). Anlam: seçili kullanıcının hesap şifresi isteniyor.
shell-greeter-password = Şifre

# Nerede: Giriş ekranı, istem alanının yer tutucusu; sistemde listelenecek kullanıcı bulunamadığında (greeter/src/view.rs). Anlam: giriş yapılacak hesabın adı elle yazılır.
shell-greeter-username = Kullanıcı adı

# Nerede: Giriş ekranı, sol alttaki düğme; tıklanınca sıradaki oturuma geçer (greeter/src/view.rs). Anlam: giriş yapılınca başlatılacak masaüstü oturumu.
# Değişkenler: $name (oturumun adı, ör. "Nuxut", "Hyprland").
shell-greeter-session = Oturum: { $name }

# Nerede: Giriş ekranı, sağ alttaki düğme, güç düğmelerinin solunda (greeter/src/view.rs). Anlam: şifrenin yazıldığı klavye düzeni; birden çok düzen varsa tıklanınca sıradakine geçer.
# Değişkenler: $layout (düzen kodu büyük harfle, ör. "TR", "US"; çevrilmez).
# [max: 12]
shell-greeter-layout = { $layout }
