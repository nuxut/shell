//! Yerleşim → Hyprland kuralları (ADR-0005 §4, §5.3, §6.2). Saf.
//!
//! İki arka uç (ADR-0003): Lua (`hl.monitor{…}`; dosya `nuxut/displays.lua`, çalışma anı
//! `hyprctl eval`) ve hyprlang (`monitor = …`; dosya `nuxut-displays.conf`, çalışma anı
//! `[[BATCH]]keyword monitor …`). Aynı monitör için dosya ve çalışma anı **aynı** `output`
//! seçicisini kullanır (Hyprland aynı seçicili kuralı günceller).
//!
//! **Kaçış:** Lua'ya giren her dize `hypr_config`'ın [`lua_str`]'inden geçer; sayılar tiplidir.
//! hyprlang'da değer [`safe_value`] olmalıdır; açıklamada `,`/`#`/`;` varsa bağlayıcı adına
//! düşülür (bağlayıcı adı zaten `[A-Za-z0-9._-]`).

use std::fmt::Write as _;

use super::model::{DisplayConfig, DisplayLayout, Hardware, Monitor, scale_str};
use crate::services::hypr_config::control::keyword_batch;
use crate::services::hypr_config::generate::safe_value;
use crate::services::hypr_config::lua::lua_str;
use crate::services::hypr_config::sync::ConfigKind;

/// Üretilen dosyaların başlıkları (sahiplik denetimi: `write_owned`).
pub const LUA_HEADER: &str = "-- Nuxut — ";
pub const CONF_HEADER: &str = "# Nuxut — ";
/// Lua modül adı (`require("nuxut.displays")` → `<ayar dizini>/nuxut/displays.lua`).
pub const LUA_MODULE: &str = "nuxut.displays";
/// Ölü adam zamanlayıcısının Lua global adı (ADR-0005 §5.3).
pub const GUARD_GLOBAL: &str = "NUXUT_DISPLAY_GUARD";

/// Üretilen dosyanın ana ayar dizinine göre yolu.
pub fn generated_rel_path(kind: ConfigKind) -> &'static str {
    match kind {
        ConfigKind::Lua => "nuxut/displays.lua",
        ConfigKind::Hyprlang => "nuxut-displays.conf",
    }
}

/// Kuralın `output` seçicisi: kalıcı kimlikte `desc:<açıklama>`, değilse bağlayıcı adı.
pub fn selector(m: &Monitor, kind: ConfigKind) -> String {
    let desc_ok = m.id_stable
        && m.description_exact
        && !m.description.is_empty()
        && match kind {
            ConfigKind::Lua => true,
            ConfigKind::Hyprlang => !m.description.contains([',', '#', ';']) && safe_value(&m.description),
        };
    if desc_ok { format!("desc:{}", m.description) } else { m.connector.clone() }
}

/// Uygulama sırası: önce yerleşenler (açılanlar), sonra yansıtanlar, en son kapananlar —
/// ara durumda tüm monitörler kapalı kalmasın (ADR-0005 §4).
fn ordered(layout: &DisplayLayout) -> Vec<&DisplayConfig> {
    let mut v: Vec<&DisplayConfig> = layout.displays.iter().collect();
    v.sort_by_key(|d| match (d.enabled, d.mirror_of.is_some()) {
        (true, false) => 0,
        (true, true) => 1,
        (false, _) => 2,
    });
    v
}

fn primary_connector<'a>(layout: &DisplayLayout, hw: &'a Hardware) -> Option<&'a str> {
    layout.primary.as_deref().and_then(|k| hw.by_key(k)).map(|m| m.connector.as_str())
}

/// `hl.monitor({…})` çağrıları (bağlı olmayan anahtarlar atlanır).
pub fn lua_rules(layout: &DisplayLayout, hw: &Hardware) -> Vec<String> {
    let mut out = Vec::new();
    for d in ordered(layout) {
        let Some(m) = hw.by_key(&d.key) else { continue };
        let output = lua_str(&selector(m, ConfigKind::Lua));
        if !d.enabled {
            out.push(format!("hl.monitor({{ output = {output}, disabled = true }})"));
            continue;
        }
        let mut s = format!(
            "hl.monitor({{ output = {output}, mode = {}, position = {}, scale = {}, transform = {}, vrr = {}",
            lua_str(&d.mode.rule()),
            lua_str(&format!("{}x{}", d.pos.0, d.pos.1)),
            scale_str(d.scale_120),
            d.transform.get(),
            d.vrr.hypr()
        );
        if let Some(target) = d.mirror_of.as_deref().and_then(|k| hw.by_key(k)) {
            let _ = write!(s, ", mirror = {}", lua_str(&target.connector));
        }
        s.push_str(" })");
        out.push(s);
    }
    if let Some(p) = primary_connector(layout, hw) {
        out.push(format!("hl.config({{ cursor = {{ default_monitor = {} }} }})", lua_str(p)));
    }
    out
}

/// `monitor = …` değerleri ve `cursor:default_monitor` (anahtar, değer).
pub fn conf_rules(layout: &DisplayLayout, hw: &Hardware) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for d in ordered(layout) {
        let Some(m) = hw.by_key(&d.key) else { continue };
        let output = selector(m, ConfigKind::Hyprlang);
        let value = if !d.enabled {
            format!("{output}, disable")
        } else {
            let mut v = format!(
                "{output}, {}, {}x{}, {}, transform, {}, vrr, {}",
                d.mode.rule(),
                d.pos.0,
                d.pos.1,
                scale_str(d.scale_120),
                d.transform.get(),
                d.vrr.hypr()
            );
            if let Some(target) = d.mirror_of.as_deref().and_then(|k| hw.by_key(k)) {
                let _ = write!(v, ", mirror, {}", target.connector);
            }
            v
        };
        if safe_value(&value) {
            out.push(("monitor".to_owned(), value));
        }
    }
    if let Some(p) = primary_connector(layout, hw) {
        out.push(("cursor:default_monitor".to_owned(), p.to_owned()));
    }
    out
}

/// Kalıcı dosyanın metni.
pub fn file_text(layout: &DisplayLayout, hw: &Hardware, kind: ConfigKind) -> String {
    match kind {
        ConfigKind::Lua => {
            // i18n: tanı üretilen Lua dosyasının başlık yorumu (dosya içeriği)
            let mut t = format!(
                "{LUA_HEADER}nuxut-shell tarafından üretildi (Ayarlar → Ekranlar); her değişiklikte üzerine yazılır.\n\
                 -- Elle düzenlemeyin. nuxut.lua bu dosyayı pcall(require, \"{LUA_MODULE}\") ile yükler.\n\n\
                 local ok, err = pcall(function()\n"
            );
            for r in lua_rules(layout, hw) {
                let _ = writeln!(t, "  {r}");
            }
            t.push_str(
                "end)\nif not ok then\n  hl.notification.create({ text = \"Nuxut ekran ayarları uygulanamadı: \" .. tostring(err), timeout = 8000 })\nend\n",
            );
            t
        }
        ConfigKind::Hyprlang => {
            // i18n: tanı üretilen conf dosyasının başlık yorumu (dosya içeriği)
            let mut t = format!(
                "{CONF_HEADER}nuxut-shell tarafından üretildi (Ayarlar → Ekranlar); her değişiklikte üzerine yazılır.\n\
                 # Elle düzenlemeyin. nuxut.conf bu dosyayı source ile yükler.\n\n"
            );
            for (k, v) in conf_rules(layout, hw) {
                let _ = writeln!(t, "{k} = {v}");
            }
            t
        }
    }
}

/// Ölü adam anahtarı kurulumu (Lua): süre dolunca `previous` kuralları uygulanır.
#[derive(Debug, Clone, PartialEq)]
pub struct Guard<'a> {
    pub previous: &'a DisplayLayout,
    pub timeout_ms: u64,
}

/// Lua: zamanlayıcıyı iptal eden kod (yoksa etkisiz).
pub fn lua_disarm() -> String {
    format!("if {GUARD_GLOBAL} then {GUARD_GLOBAL}:set_enabled(false); {GUARD_GLOBAL} = nil end")
}

/// Çalışma anı komutu (tek istek): Lua → `eval <tek satır>`; hyprlang → `[[BATCH]]keyword …`.
/// Lua'da varsa eski zamanlayıcı iptal edilir, `guard` verilirse yenisi kurulur. hyprlang'da
/// zamanlayıcı yoktur (bekçi süreç ayrıca başlatılır: [`legacy_guard_exec`]). Güvensiz değer
/// varsa `None`.
pub fn runtime_command(layout: &DisplayLayout, hw: &Hardware, kind: ConfigKind, guard: Option<Guard<'_>>) -> Option<String> {
    match kind {
        ConfigKind::Lua => {
            let mut parts = Vec::new();
            match guard {
                Some(g) => {
                    // Yeni değişiklik: eski zamanlayıcı (varsa) iptal, kurallar, yeni zamanlayıcı.
                    parts.push(lua_disarm());
                    parts.extend(lua_rules(layout, hw));
                    let prev = lua_rules(g.previous, hw).join(" ");
                    parts.push(format!(
                        "{GUARD_GLOBAL} = hl.timer(function() {prev} end, {{ timeout = {}, type = \"oneshot\" }})",
                        g.timeout_ms.clamp(1000, 600_000)
                    ));
                }
                None => {
                    // Geri dönüş/kayıtlı küme: önce kurallar, zamanlayıcı EN SONDA iptal — bir kural
                    // hata verirse eval orada durur ve Hyprland'deki zamanlayıcı yine geri döner.
                    parts.extend(lua_rules(layout, hw));
                    parts.push(lua_disarm());
                }
            }
            let code = parts.join("; ");
            (!code.contains(['\n', '\r', '\0'])).then(|| format!("eval {code}"))
        }
        ConfigKind::Hyprlang => keyword_batch(&conf_rules(layout, hw)),
    }
}

/// Kurtarma: kapalı ya da yansıtan her bağlı monitörü Hyprland varsayılanıyla (`preferred`,
/// `auto`, `auto`) açan çalışma anı komutu — hiç etkin ekran kalmadığında (ör. dizüstü ekranı
/// kapalı onaylanmış, harici monitör çıkarılmış). Lua'da bekleyen zamanlayıcı da en sonda iptal.
/// Açılacak monitör yoksa `None`.
pub fn fallback_command(hw: &Hardware, kind: ConfigKind) -> Option<String> {
    let targets: Vec<&Monitor> = hw.monitors.iter().filter(|m| !m.enabled || m.mirror_of.is_some()).collect();
    if targets.is_empty() {
        return None;
    }
    match kind {
        ConfigKind::Lua => {
            let mut parts: Vec<String> = targets
                .iter()
                .map(|m| {
                    format!(
                        "hl.monitor({{ output = {}, mode = \"preferred\", position = \"auto\", scale = \"auto\" }})",
                        lua_str(&m.connector)
                    )
                })
                .collect();
            parts.push(lua_disarm());
            Some(format!("eval {}", parts.join("; ")))
        }
        ConfigKind::Hyprlang => {
            let rules: Vec<(String, String)> =
                targets.iter().map(|m| ("monitor".to_owned(), format!("{}, preferred, auto, auto", m.connector))).collect();
            keyword_batch(&rules)
        }
    }
}

/// hyprlang ölü adam anahtarı:Hyprland'in çocuğu olarak çalışan bekçi (shell ölse de yaşar).
/// `exe` güvenli yol olmalı (`hypr_config` `safe_exe`); jeton onaltılık, örnek adı `[a-z0-9_-]`.
pub fn legacy_guard_exec(exe: &str, token: &str, deadline_unix_ms: u64, app: &str) -> Option<String> {
    let ok_token = !token.is_empty() && token.len() <= 32 && token.bytes().all(|b| b.is_ascii_hexdigit());
    let ok_app = !app.is_empty() && app.len() <= 64 && app.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_-".contains(&b));
    let ok_exe = exe.bytes().all(|b| b.is_ascii_alphanumeric() || b"/._+-".contains(&b)) && !exe.is_empty();
    (ok_token && ok_app && ok_exe).then(|| format!("dispatch exec {exe} display-guard {token} {deadline_unix_ms} {app}"))
}

#[cfg(test)]
mod tests {
    use super::super::model::fixtures::{DUAL, VM};
    use super::super::model::Transform;
    use super::*;

    const EDP: &str = "BOE|0x0BCA||eDP-1";
    const DELL: &str = "Dell Inc.|DELL U2720Q|ABC123";

    fn dual() -> (DisplayLayout, Hardware) {
        let hw = Hardware::parse(DUAL).unwrap();
        let mut l = hw.current_layout();
        l.primary = Some(DELL.into());
        (l, hw)
    }

    #[test]
    fn lua_golden() {
        let (mut l, hw) = dual();
        l.displays.iter_mut().find(|d| d.key == EDP).unwrap().scale_120 = 180;
        let text = file_text(&l, &hw, ConfigKind::Lua);
        let expected = "-- Nuxut — nuxut-shell tarafından üretildi (Ayarlar → Ekranlar); her değişiklikte üzerine yazılır.\n\
-- Elle düzenlemeyin. nuxut.lua bu dosyayı pcall(require, \"nuxut.displays\") ile yükler.\n\n\
local ok, err = pcall(function()\n  \
hl.monitor({ output = \"eDP-1\", mode = \"3840x2400@60.000\", position = \"0x0\", scale = 1.5, transform = 0, vrr = 0 })\n  \
hl.monitor({ output = \"desc:Dell Inc. DELL U2720Q ABC123\", mode = \"2560x1440@59.951\", position = \"1920x0\", scale = 1, transform = 0, vrr = 1 })\n  \
hl.config({ cursor = { default_monitor = \"DP-1\" } })\n\
end)\nif not ok then\n  hl.notification.create({ text = \"Nuxut ekran ayarları uygulanamadı: \" .. tostring(err), timeout = 8000 })\nend\n";
        assert_eq!(text, expected);
    }

    #[test]
    fn conf_golden_with_mirror_and_disable() {
        let (mut l, hw) = dual();
        l.displays.iter_mut().find(|d| d.key == EDP).unwrap().mirror_of = Some(DELL.into());
        l.displays.iter_mut().find(|d| d.key == DELL).unwrap().transform = Transform::new(1).unwrap();
        let text = file_text(&l, &hw, ConfigKind::Hyprlang);
        assert!(text.starts_with(CONF_HEADER));
        let body: Vec<&str> = text.lines().filter(|l| !l.starts_with('#') && !l.is_empty()).collect();
        assert_eq!(
            body,
            [
                "monitor = desc:Dell Inc. DELL U2720Q ABC123, 2560x1440@59.951, 1920x0, 1, transform, 1, vrr, 1",
                "monitor = eDP-1, 3840x2400@60.000, 0x0, 2, transform, 0, vrr, 0, mirror, DP-1",
                "cursor:default_monitor = DP-1",
            ]
        );
        l.displays.iter_mut().find(|d| d.key == EDP).unwrap().enabled = false;
        let rules = conf_rules(&l, &hw);
        assert_eq!(rules.last().unwrap().0, "cursor:default_monitor");
        assert_eq!(rules[rules.len() - 2].1, "eDP-1, disable", "kapananlar en sonda");
    }

    #[test]
    fn clipped_description_falls_back_to_connector() {
        let long = "Dell Inc. DELL U2720Q ABC123 ".repeat(4);
        let json = DUAL.replace("Dell Inc. DELL U2720Q ABC123", long.trim());
        let hw = Hardware::parse(&json).unwrap();
        let dell = hw.by_connector("DP-1").unwrap();
        assert!(dell.id_stable && !dell.description_exact);
        assert_eq!(selector(dell, ConfigKind::Lua), "DP-1");
        assert_eq!(selector(Hardware::parse(DUAL).unwrap().by_connector("DP-1").unwrap(), ConfigKind::Lua), "desc:Dell Inc. DELL U2720Q ABC123");
    }

    #[test]
    fn fallback_turns_on_disabled_and_mirroring() {
        let json = DUAL.replacen("\"disabled\":false", "\"disabled\":true", 1);
        let hw = Hardware::parse(&json).unwrap();
        let lua = fallback_command(&hw, ConfigKind::Lua).unwrap();
        assert!(lua.starts_with("eval hl.monitor({ output = \"eDP-1\", mode = \"preferred\"") && lua.ends_with("NUXUT_DISPLAY_GUARD = nil end"), "{lua}");
        assert!(!lua.contains("output = \"DP-1\""), "etkin monitöre dokunulmaz");
        assert_eq!(fallback_command(&hw, ConfigKind::Hyprlang).unwrap(), "[[BATCH]]keyword monitor eDP-1, preferred, auto, auto");
        assert!(fallback_command(&Hardware::parse(DUAL).unwrap(), ConfigKind::Lua).is_none());
    }

    #[test]
    fn vm_uses_connector() {
        let hw = Hardware::parse(VM).unwrap();
        let l = hw.current_layout();
        let r = lua_rules(&l, &hw);
        assert!(r[0].starts_with("hl.monitor({ output = \"Virtual-1\", mode = \"1920x1080@30.000\""), "{}", r[0]);
    }

    #[test]
    fn hostile_description_is_escaped_or_avoided() {
        let json = DUAL.replace("Dell Inc. DELL U2720Q ABC123", "Evil\\\", reload; # \\u00e7");
        let hw = Hardware::parse(&json).unwrap();
        let l = hw.current_layout();
        let lua = lua_rules(&l, &hw).join("\n");
        assert!(lua.contains("desc:Evil\\\", reload; # \\195\\167"), "{lua}");
        let conf = conf_rules(&l, &hw);
        assert!(conf.iter().any(|(_, v)| v.starts_with("DP-1, ")), "hyprlang'da bağlayıcı adına düşer: {conf:?}");
        assert!(keyword_batch(&conf).is_some());
    }

    #[test]
    fn runtime_commands() {
        let (l, hw) = dual();
        let mut prev = l.clone();
        prev.displays[0].scale_120 = 240;
        let eval = runtime_command(&l, &hw, ConfigKind::Lua, Some(Guard { previous: &prev, timeout_ms: 20_000 })).unwrap();
        assert!(eval.starts_with("eval if NUXUT_DISPLAY_GUARD then NUXUT_DISPLAY_GUARD:set_enabled(false); NUXUT_DISPLAY_GUARD = nil end; hl.monitor("));
        assert!(eval.ends_with("end, { timeout = 20000, type = \"oneshot\" })"));
        assert!(!eval.contains('\n'));
        let batch = runtime_command(&l, &hw, ConfigKind::Hyprlang, None).unwrap();
        assert!(batch.starts_with("[[BATCH]]keyword monitor eDP-1, 3840x2400@60.000, 0x0, 2,"), "{batch}");
        assert!(batch.ends_with(";keyword cursor:default_monitor DP-1"));
        assert_eq!(
            legacy_guard_exec("/usr/bin/nuxut-shell", "a1b2", 17, "nuxut-shell").unwrap(),
            "dispatch exec /usr/bin/nuxut-shell display-guard a1b2 17 nuxut-shell"
        );
        assert!(legacy_guard_exec("/bin/x;rm", "a1", 1, "nuxut-shell").is_none());
        assert!(legacy_guard_exec("/bin/x", "zz", 1, "nuxut-shell").is_none());
    }

    /// Üretilen Lua/conf dosyalarını gerçek Hyprland'in statik denetiminden geçirir
    /// (`Hyprland --verify-config`; compositor açılmaz, çalışan oturuma bağlanılmaz).
    /// Hyprland kurulu değilse atlanır.
    #[test]
    fn hyprland_verify_config() {
        let Some(bin) = ["/usr/bin/Hyprland", "/usr/local/bin/Hyprland"].into_iter().find(|p| std::path::Path::new(p).exists()) else {
            return;
        };
        let (mut l, hw) = dual();
        l.displays.iter_mut().find(|d| d.key == EDP).unwrap().mirror_of = Some(DELL.into());
        let tmp = tempfile::tempdir().unwrap();
        let prev = hw.current_layout();
        let eval = runtime_command(&l, &hw, ConfigKind::Lua, Some(Guard { previous: &prev, timeout_ms: 20_000 })).unwrap();
        // Dosyadaki pcall hataları yutar: kurallar ve eval kodu ayrıca çıplak denetlenir.
        let bare = lua_rules(&l, &hw).join("\n");
        let cases = [
            ("hyprland.lua", file_text(&l, &hw, ConfigKind::Lua)),
            ("hyprland.lua", bare),
            // --verify-config hl.timer'ı çalıştıramaz (olay döngüsü yok, süreç çöker): bekçili kod
            // yalnızca sözdizimi için çağrılmayan bir fonksiyonda; bekçisiz kod çalıştırılarak.
            ("hyprland.lua", format!("local function _guarded() {} end", eval.trim_start_matches("eval "))),
            ("hyprland.lua", runtime_command(&l, &hw, ConfigKind::Lua, None).unwrap().trim_start_matches("eval ").to_owned()),
            (
                "hyprland.lua",
                fallback_command(&Hardware::parse(&DUAL.replacen("\"disabled\":false", "\"disabled\":true", 1)).unwrap(), ConfigKind::Lua)
                    .unwrap()
                    .trim_start_matches("eval ")
                    .to_owned(),
            ),
            ("hyprland.conf", file_text(&l, &hw, ConfigKind::Hyprlang)),
        ];
        for (i, (main, body)) in cases.into_iter().enumerate() {
            let kind = format!("{i}:{main}");
            let path = tmp.path().join(main);
            std::fs::write(&path, &body).unwrap();
            let out = std::process::Command::new(bin)
                .arg("--verify-config")
                .arg("-c")
                .arg(&path)
                .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
                .env_remove("WAYLAND_DISPLAY")
                .env("XDG_RUNTIME_DIR", tmp.path())
                .env("HOME", tmp.path())
                .env("XDG_CONFIG_HOME", tmp.path())
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
            assert!(text.contains("config ok"), "{kind} {:?}: {text}\n---\n{body}", out.status);
        }
    }
}
