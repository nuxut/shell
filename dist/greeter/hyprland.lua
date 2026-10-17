-- Nuxut giriş ekranının compositor ayarı (greetd default_session, kullanıcı `greeter`).
-- Kurulum: /usr/share/nuxut/greeter/hyprland.lua. Klavye düzeni başlatıcının ürettiği
-- `input.lua`'dan (nuxut-greeter-session). Karar ve tehdit modeli: docs/adr/0010-login-screen.md.

-- ÖNCE zararsız bir bind: Hyprland hiç bind kayıtlı değilken (ya da Lua hatasında) "acil durum"
-- bind'lerini açar (SUPER+Q terminal, SUPER+R hyprland-run, SUPER+M çıkış) — giriş ekranında
-- bunlar greeter kullanıcısıyla program çalıştırmak demek. Bu satır dosyanın geri kalanı hata
-- verse bile kayıtlı kalır.
hl.bind("SUPER + SHIFT + CTRL + ALT + F24", hl.dsp.exec_cmd("true"), { description = "nuxut:greeter-noop" })

hl.config({
    misc = {
        disable_hyprland_logo = true,
        disable_splash_rendering = true,
        force_default_wallpaper = 0,
        disable_watchdog_warning = true,
        disable_autoreload = true,
        -- Katman yüzeyi klavyeyi bırakmasın (tıklama başka yüzeye odak vermesin).
        layers_hog_keyboard_focus = true,
    },
    animations = { enabled = false },
    ecosystem = { no_update_news = true, no_donation_nag = true },
    general = { border_size = 0 },
})

-- Klavye düzeni (localectl'den; başlatıcı doğrulanmış değerlerle yazar). Yoksa "us".
pcall(require, "input")
-- Ekran okuyucu (yayımlanan greeter.toml screen_reader = true ise başlatıcı yazar; yoksa boş).
pcall(require, "a11y")

-- Giriş ekranı bitince (oturum başlatıldı ya da sonlandırıldı) compositor da çıkar; greetd
-- oturumu ondan sonra açar.
hl.on("hyprland.start", function()
    hl.exec_cmd("/usr/bin/nuxut-greeter; hyprctl dispatch 'hl.dsp.exit()'")
end)
