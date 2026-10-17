//! Lua arka ucu (Hyprland ≥ 0.55, ADR-0003): `nuxut.lua` üretimi, kaçış ve kullanıcının
//! `hyprland.lua` zincirinde dahil etme satırının taranması. Kısayollar ve kurallar temel
//! katmanda ([`super::base`], ADR-0006).
//!
//! **Kaçış kuralı:** Lua koduna giren her değer ya sayı/bool (üreticide sınırlı, sonlu) ya da
//! [`lua_str`]'den geçmiş dizedir. Dize birleştirme (`..`), uzun dizeler (`[[ ]]`) ve dışarıdan
//! gelen tanımlayıcı yoktur; tablo anahtarları kodda sabittir.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::Read;
use std::path::{Path, PathBuf};

use super::generate::{
    CURVES, GenerateInput, Generated, anim_plan, hypr_color, polkit_start_command, polkit_stop_command, safe_env, safe_exe, session_path_dir, start_command, stop_command,
};
use super::input::{HyprValue, InputSettings, float_text};
use super::parse::{ScanLimits, launches_shell, normalize};

/// Modül adı: `require("nuxut")` → `<ayar dizini>/nuxut.lua`.
pub const MODULE: &str = "nuxut";
/// Kullanıcının dosyasına eklenen satır. `pcall`: dosya yoksa ya da bozuksa kullanıcının ayarı
/// yine çalışır (bulunamayan modül tüm ayarı durdururdu).
pub const INCLUDE_LINE: &str = "pcall(require, \"nuxut\")";

/// Lua dize sabiti: `"…"`; `\\`, `"` ve yazdırılamayan/ASCII dışı her bayt `\ddd` (Lua 5.4).
pub fn lua_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for &b in s.as_bytes() {
        match b {
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            0x20..=0x7e => out.push(b as char),
            _ => {
                let _ = write!(out, "\\{b:03}");
            }
        }
    }
    out.push('"');
    out
}

/// Sonlu sayı (en çok 4 ondalık); sonlu değilse 0.
fn lnum(v: f32) -> String {
    let v = if v.is_finite() { v } else { 0.0 };
    let r = (f64::from(v) * 10_000.0).round() / 10_000.0;
    if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{r}") }
}

fn lint(v: f32, max: f32) -> String {
    let v = if v.is_finite() { v.clamp(0.0, max) } else { 0.0 };
    format!("{}", v.round() as i64)
}

/// Hyprland hızı (desisaniye, bir ondalık, [0.5, 30]); hyprlang'daki `deciseconds` ile aynı.
fn lua_speed(ms: f32) -> String {
    lnum(((ms / 100.0) * 10.0).round().clamp(5.0, 300.0) / 10.0)
}

/// Yazılı `[input]` anahtarlarının listesi bu önekli yorum satırında: küme değişirse (özellikle bir
/// anahtar kalkınca) canlı uygulama `eval` değil yeniden okumadır ([`super::sync::plan_apply_lua`]) —
/// `eval` kaldırılan değeri geri almaz, yeniden okuma kullanıcının değerine döner.
pub const INPUT_KEYS_MARKER: &str = "-- nuxut-input:";

/// `input = { kb_layout = "tr,us", …, touchpad = { tap_to_click = true } }`; yazılı anahtar yoksa `None`.
fn lua_input_table(input: &GenerateInput) -> Option<String> {
    let lines = input.input.as_ref().map(InputSettings::lines).unwrap_or_default();
    let value = |v: &HyprValue| match v {
        HyprValue::Str(s) => lua_str(s),
        HyprValue::Int(i) => i.to_string(),
        HyprValue::Float(f) => float_text(*f),
        HyprValue::Bool(b) => b.to_string(),
    };
    let (mut top, mut touchpad) = (Vec::new(), Vec::new());
    for l in &lines {
        match l.key.lua.strip_prefix("touchpad.") {
            Some(name) => touchpad.push(format!("{name} = {}", value(&l.value))),
            None => top.push(format!("{} = {}", l.key.lua, value(&l.value))),
        }
    }
    if !touchpad.is_empty() {
        top.push(format!("touchpad = {{ {} }}", touchpad.join(", ")));
    }
    (!top.is_empty()).then(|| format!("input = {{ {} }}", top.join(", ")))
}

/// Ayarlar → `nuxut.lua` metni + `eval` kodu (değerler; bind'siz).
pub fn generate_lua(input: &GenerateInput) -> Generated {
    let exe = safe_exe(&input.shell_exe);
    let mut values = String::new(); // hl.config + eğriler + animasyonlar (eval ile de gider)

    // hl.config({...}) tablosu.
    let mut decoration = Vec::new();
    let mut general = Vec::new();
    let mut animations = None;
    if let Some(look) = &input.look {
        decoration.push(format!("rounding = {}", lint(look.rounding, 1000.0)));
        decoration.push(match look.shadow {
            Some(s) => format!(
                "shadow = {{ enabled = true, range = {}, render_power = 3, offset = {{ 0, {} }}, color = {} }}",
                lint(s.range, 1000.0),
                lint(s.offset_y, 1000.0),
                lua_str(&hypr_color(s.color))
            ),
            None => "shadow = { enabled = false }".into(),
        });
        decoration.push(match look.blur {
            Some(b) => format!("blur = {{ enabled = true, size = {}, passes = {} }}", b.size.clamp(1, 32), b.passes.clamp(1, 4)),
            None => "blur = { enabled = false }".into(),
        });
    }
    if let Some(w) = &input.windows {
        general.push(format!("gaps_in = {}", w.gaps_in.min(64)));
        general.push(format!("gaps_out = {}", w.gaps_out.min(128)));
        general.push(format!("border_size = {}", w.border.min(16)));
        general.push(format!(
            "col = {{ active_border = {}, inactive_border = {} }}",
            lua_str(&hypr_color(w.active_border)),
            lua_str(&hypr_color(w.inactive_border))
        ));
    }
    let mut curves = Vec::new();
    let mut anims = Vec::new();
    if let Some(m) = &input.motion {
        let plan = anim_plan(m);
        animations = Some(plan.is_some());
        if let Some(plan) = plan {
            for (name, b) in CURVES {
                curves.push(format!(
                    "hl.curve({}, {{ type = \"bezier\", points = {{ {{ {}, {} }}, {{ {}, {} }} }} }})",
                    lua_str(name),
                    lnum(b.x1),
                    lnum(b.y1),
                    lnum(b.x2),
                    lnum(b.y2)
                ));
            }
            for a in plan {
                anims.push(match a.ms {
                    Some(ms) => {
                        let style = if a.style.is_empty() { String::new() } else { format!(", style = {}", lua_str(a.style)) };
                        format!(
                            "hl.animation({{ leaf = {}, enabled = true, speed = {}, bezier = {}{style} }})",
                            lua_str(a.leaf),
                            lua_speed(ms),
                            lua_str(a.curve)
                        )
                    }
                    None => format!("hl.animation({{ leaf = {}, enabled = false }})", lua_str(a.leaf)),
                });
            }
        }
    }
    let mut sections = Vec::new();
    if !general.is_empty() {
        sections.push(format!("general = {{ {} }}", general.join(", ")));
    }
    if !decoration.is_empty() {
        sections.push(format!("decoration = {{ {} }}", decoration.join(", ")));
    }
    if let Some(e) = animations {
        sections.push(format!("animations = {{ enabled = {e} }}"));
    }
    if let Some(t) = lua_input_table(input) {
        sections.push(t);
    }
    if input.hide_defaults {
        sections.push("misc = { disable_hyprland_logo = true, force_default_wallpaper = 0 }".into());
    }
    let mut value_lines = Vec::new();
    if !sections.is_empty() {
        value_lines.push(format!("hl.config({{ {} }})", sections.join(", ")));
    }
    value_lines.extend(curves);
    value_lines.extend(anims);
    for l in &value_lines {
        let _ = writeln!(values, "  {l}");
    }

    let mut text = String::from(
        "-- Nuxut — nuxut-shell tarafından üretildi; her değişiklikte üzerine yazılır.\n\
         -- Elle düzenlemeyin: kendi ayarlarınızı hyprland.lua'ya yazın. Nuxut'un bir grubu\n\
         -- yönetmesini istemiyorsanız shell.toml [compositor] sync_* anahtarını kapatın.\n\
         -- hyprland.lua'daki pcall(require, \"nuxut\") satırı bu dosyayı yükler.\n",
    );
    let input_keys: Vec<&str> = input.input.as_ref().map(InputSettings::lines).unwrap_or_default().iter().map(|l| l.key.lua).collect();
    if !input_keys.is_empty() {
        let _ = writeln!(text, "{INPUT_KEYS_MARKER} {}", input_keys.join(" "));
    }
    text.push_str("\nlocal ok, err = pcall(function()\n");
    // Kabuğun dizini oturumun PATH'inde (generate::session_path_dir); Lua ortamı canlı okur:
    // dizin zaten varsa eklenmez (yeniden yüklemede büyümez), `true` = systemd/D-Bus'a da.
    if let Some(dir) = session_path_dir(&input.shell_exe) {
        let _ = writeln!(
            text,
            "  local path = os.getenv(\"PATH\") or \"\"\n  if not (\":\" .. path .. \":\"):find({}, 1, true) then hl.env(\"PATH\", {} .. \":\" .. path, true) end",
            lua_str(&format!(":{dir}:")),
            lua_str(dir)
        );
    }
    // Oturum ortamı (portal) ve imleç ortamı (birleşik görünüm; eval'e girmez — canlı imleç `setcursor` ile).
    for (k, v) in input.env.iter().filter(|(k, v)| safe_env(k, v)) {
        let _ = writeln!(text, "  hl.env({}, {})", lua_str(k), lua_str(v));
    }
    text.push_str(&values);
    text.push_str(
        "end)\nif not ok then\n  hl.notification.create({ text = \"Nuxut ayarları uygulanamadı: \" .. tostring(err), timeout = 8000 })\nend\n",
    );
    if input.displays.is_some() {
        // Ekranlar (ADR-0005 §6.2): ayrı dosya `nuxut/displays.lua`; kendi pcall'ı var.
        text.push_str("\n-- Ekranlar (Ayarlar → Ekranlar)\npcall(require, \"nuxut.displays\")\n");
    }
    if input.launch {
        let _ = write!(
            text,
            "\n-- Oturum başında kabuğu başlat (systemd kullanıcı birimi; yoksa doğrudan).\n-- polkit ajanı yalnızca birimi etkinse (kendi ajanınızı kullanıyorsanız dokunulmaz).\nhl.on(\"hyprland.start\", function() hl.exec_cmd({}) hl.exec_cmd({}) end)\nhl.on(\"hyprland.shutdown\", function() hl.exec_cmd({}) hl.exec_cmd({}) end)\n", // i18n: tanı üretilen Lua dosyasının yorum ve bildirim metni (dosya içeriği)
            lua_str(&start_command(exe)),
            lua_str(&polkit_start_command()),
            lua_str(&stop_command()),
            lua_str(&polkit_stop_command())
        );
    }
    let eval = (!value_lines.is_empty()).then(|| value_lines.join(" "));
    Generated { text, keywords: Vec::new(), eval }
}

// ---------------------------------------------------------------------------------------
// Kullanıcının Lua zinciri taraması (saf, sınırlı)

/// Lua kaynağından yorumları atar (`--` satır, `--[[ ]]`/`--[==[ ]==]` blok); dize
/// sabitlerinin içindeki `--` yorum sayılmaz. Yorumlanmış metin ayrıca döner (kapatılmış
/// dahil etme satırını bulmak için).
pub fn strip_lua_comments(src: &str) -> (String, String) {
    let b = src.as_bytes();
    let mut code = String::with_capacity(src.len());
    let mut comments = String::new();
    let mut i = 0;
    let mut in_str: Option<u8> = None;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = in_str {
            code.push(c as char);
            if c == b'\\' && i + 1 < b.len() {
                code.push(b[i + 1] as char);
                i += 2;
                continue;
            }
            if c == q || c == b'\n' {
                in_str = None;
            }
            i += 1;
            continue;
        }
        if c == b'"' || c == b'\'' {
            in_str = Some(c);
            code.push(c as char);
            i += 1;
            continue;
        }
        if c == b'-' && b.get(i + 1) == Some(&b'-') {
            // Blok yorum mu?
            let rest = &b[i + 2..];
            if rest.first() == Some(&b'[') {
                let eqs = rest[1..].iter().take_while(|&&x| x == b'=').count();
                if rest.get(1 + eqs) == Some(&b'[') {
                    let close: Vec<u8> = std::iter::once(b']').chain(std::iter::repeat_n(b'=', eqs)).chain(std::iter::once(b']')).collect();
                    let start = i + 2 + 2 + eqs;
                    let end = find(&b[start..], &close).map_or(b.len(), |p| start + p + close.len());
                    comments.push_str(&String::from_utf8_lossy(&b[start.min(b.len())..end.min(b.len())]));
                    comments.push('\n');
                    i = end;
                    continue;
                }
            }
            let end = b[i..].iter().position(|&x| x == b'\n').map_or(b.len(), |p| i + p);
            comments.push_str(&String::from_utf8_lossy(&b[i + 2..end]));
            comments.push('\n');
            i = end;
            continue;
        }
        code.push(c as char);
        i += 1;
    }
    // Bayt bayt kopyalandı: ASCII dışı baytlar bozulmuş olabilir; yalnızca ASCII desenler aranır.
    (code, comments)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// `require` edilen modül adları (dize sabitiyle verilenler).
pub fn required_modules(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = code;
    while let Some(p) = rest.find("require") {
        let after = &rest[p + "require".len()..];
        rest = after;
        // `require "x"`, `require("x")`, `require('x')`, `pcall(require, "x")`
        let t = after.trim_start_matches([' ', '\t', '(', ',']);
        let Some(q) = t.chars().next().filter(|c| *c == '"' || *c == '\'') else { continue };
        let body = &t[1..];
        if let Some(end) = body.find(q)
            && end <= 256
        {
            out.push(body[..end].to_owned());
        }
    }
    out
}

/// `dofile("...")`/`loadfile("...")` yolları.
fn dofiles(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    for kw in ["dofile", "loadfile"] {
        let mut rest = code;
        while let Some(p) = rest.find(kw) {
            let t = rest[p + kw.len()..].trim_start_matches([' ', '\t', '(']);
            rest = &rest[p + kw.len()..];
            if let Some(q) = t.chars().next().filter(|c| *c == '"' || *c == '\'')
                && let Some(end) = t[1..].find(q)
            {
                out.push(t[1..1 + end].to_owned());
            }
        }
    }
    out
}

/// Lua taraması sonucu (hyprlang taramasının Lua karşılığı; yerler dosya yolu olarak).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LuaReport {
    pub files: Vec<PathBuf>,
    /// Modülü yükleyen (yorum olmayan) dosya.
    pub included_in: Option<usize>,
    /// Yalnızca yorum içinde dahil etme var (kullanıcı kapatmış).
    pub commented_in: Option<usize>,
    /// Kullanıcının kendi kabuk başlatıcısı (`hl.exec_cmd("…nuxut-shell…")`).
    pub external_launch_in: Option<usize>,
    pub truncated: bool,
}

/// `root`'tan başlayarak `require`/`dofile` zincirini tarar. `require` adı `a.b` →
/// `<dizin>/a/b.lua` ya da `<dizin>/a/b/init.lua` (Hyprland'in `package.path`'i).
pub fn scan_lua(root: &Path, home: Option<&Path>, limits: ScanLimits) -> LuaReport {
    let dir = root.parent().map(Path::to_path_buf).unwrap_or_default();
    let target = dir.join(format!("{MODULE}.lua"));
    let mut report = LuaReport::default();
    let mut seen = HashSet::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((path, depth)) = stack.pop() {
        if depth > limits.depth || report.files.len() >= limits.files {
            report.truncated = true;
            continue;
        }
        let key = path.canonicalize().unwrap_or_else(|_| normalize(&path));
        if !seen.insert(key) {
            continue;
        }
        let Some(text) = read(&path, limits.file_bytes, &mut report.truncated) else {
            if depth == 0 {
                report.truncated = true;
            }
            continue;
        };
        let idx = report.files.len();
        report.files.push(path.clone());
        let (code, comments) = strip_lua_comments(&text);
        let is_target = |p: &Path| normalize(p) == normalize(&target);
        let mods = required_modules(&code);
        if mods.iter().any(|m| m == MODULE) && report.included_in.is_none() {
            report.included_in = Some(idx);
        }
        if report.commented_in.is_none() && required_modules(&comments).iter().any(|m| m == MODULE) {
            report.commented_in = Some(idx);
        }
        for f in dofiles(&code) {
            let p = resolve(&f, &dir, home);
            if p.as_deref().is_some_and(is_target) {
                report.included_in.get_or_insert(idx);
            } else if let Some(p) = p {
                stack.push((p, depth + 1));
            }
        }
        if report.external_launch_in.is_none() && code.contains("exec_cmd") && code_strings(&code).iter().any(|s| launches_shell(s)) {
            report.external_launch_in = Some(idx);
        }
        for m in mods {
            // Nuxut'un kendi modülleri (`nuxut`, `nuxut.base`, `nuxut.displays`) taranmaz.
            if m == MODULE || m.starts_with("nuxut.") || m.is_empty() || m.contains(['/', '\\']) && !m.starts_with('/') && !m.starts_with("./") && !m.starts_with("~/") {
                continue;
            }
            if let Some(p) = resolve(&m, &dir, home).filter(|_| m.starts_with('/') || m.starts_with("./") || m.starts_with("~/")) {
                stack.push((p, depth + 1));
                continue;
            }
            let rel = m.replace('.', "/");
            for cand in [dir.join(format!("{rel}.lua")), dir.join(&rel).join("init.lua")] {
                if cand.is_file() {
                    stack.push((cand, depth + 1));
                    break;
                }
            }
        }
    }
    report
}

fn read(path: &Path, max: u64, truncated: &mut bool) -> Option<String> {
    let mut buf = Vec::new();
    std::fs::File::open(path).ok()?.take(max + 1).read_to_end(&mut buf).ok()?;
    if buf.len() as u64 > max {
        *truncated = true;
        return None;
    }
    Some(String::from_utf8_lossy(&buf).into_owned())
}

fn resolve(raw: &str, dir: &Path, home: Option<&Path>) -> Option<PathBuf> {
    super::parse::resolve_path(raw, dir, home)
}

/// Koddaki dize sabitleri (kaba: `"…"` ve `'…'`).
fn code_strings(code: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = code.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'"' || b[i] == b'\'' {
            let q = b[i];
            let start = i + 1;
            let mut j = start;
            while j < b.len() && b[j] != q && b[j] != b'\n' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            out.push(String::from_utf8_lossy(&b[start..j.min(b.len())]).into_owned());
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fs;

    use nuxut::Theme;

    use super::*;
    use crate::services::hypr_config::generate::{BlurLook, LookGroup, MotionGroup, WindowsGroup};

    #[test]
    fn lua_strings_are_escaped() {
        assert_eq!(lua_str("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(lua_str("x\ny]]"), "\"x\\010y]]\"");
        assert_eq!(lua_str("ç"), "\"\\195\\167\"");
        // Enjeksiyon denemesi tek bir dize sabiti olarak kalır.
        for evil in ["\") os.execute(\"rm -rf ~\") --", "a\\\" b", "\\", "]]..x", "çğ\u{0}\u{7f}\r\n"] {
            let s = lua_str(evil);
            assert!(s.starts_with('"') && s.ends_with('"') && s.len() >= 2);
            let inner = &s.as_bytes()[1..s.len() - 1];
            assert!(inner.iter().all(|b| (0x20..=0x7e).contains(b)));
            // Lua 5.4 kuralıyla çöz: tırnak yalnızca kaçışlı görünür ve sonuç özgün bayt dizisi.
            let mut out = Vec::new();
            let mut i = 0;
            while i < inner.len() {
                match inner[i] {
                    b'"' => panic!("kaçışsız tırnak: {s}"),
                    b'\\' => {
                        let n = inner[i + 1];
                        if n.is_ascii_digit() {
                            let d = std::str::from_utf8(&inner[i + 1..i + 4]).unwrap();
                            out.push(d.parse::<u8>().unwrap());
                            i += 4;
                        } else {
                            out.push(n);
                            i += 2;
                        }
                    }
                    b => {
                        out.push(b);
                        i += 1;
                    }
                }
            }
            assert_eq!(out, evil.as_bytes(), "{s}");
        }
    }

    fn input() -> GenerateInput {
        let theme = Theme::default();
        GenerateInput {
            launch: true,
            shell_exe: "/home/u/.local/bin/nuxut-shell".into(),
            look: Some(LookGroup::from_theme(&theme, Some(BlurLook { size: 8, passes: 2 }))),
            motion: Some(MotionGroup::from_theme(&theme)),
            windows: Some(WindowsGroup::from_theme(&theme, 4, 8, 2)),
            hide_defaults: true,
            displays: None,
            env: Vec::new(),
            input: None,
        }
    }

    #[test]
    fn generated_module_shape() {
        let g = generate_lua(&input());
        let t = &g.text;
        assert!(t.starts_with("-- Nuxut — "));
        assert!(t.contains("local ok, err = pcall(function()\n"));
        assert!(t.contains("hl.config({ general = { gaps_in = 4, gaps_out = 8, border_size = 2, col = { active_border = \"rgba("));
        assert!(t.contains("decoration = { rounding = 22, shadow = { enabled = true, range = "));
        assert!(t.contains("animations = { enabled = true }"));
        assert!(t.contains(", misc = { disable_hyprland_logo = true, force_default_wallpaper = 0 } })\n"));
        // Eğriler iki arka uçta aynı (hyprlang `bezier` ile): vurgulu yavaşlama, taşma yok.
        assert!(t.contains("hl.curve(\"nuxut_decel\", { type = \"bezier\", points = { { 0.05, 0.7 }, { 0.1, 1 } } })"));
        assert!(t.contains("hl.animation({ leaf = \"windowsIn\", enabled = true, speed = 2.5, bezier = \"nuxut_decel\", style = \"popin 92%\" })"));
        assert!(t.contains("hl.animation({ leaf = \"workspaces\", enabled = true, speed = 2.7, bezier = \"nuxut_decel\", style = \"slide\" })"));
        assert!(t.contains("hl.animation({ leaf = \"border\", enabled = true, speed = 2, bezier = \"nuxut_std\" })"));
        assert!(!t.contains("spring"), "yay eğrisi yok (ADR-0006 §4)");
        // Kısayollar ve kurallar temel katmanda.
        assert!(!t.contains("hl.bind(") && !t.contains("hl.window_rule("));
        assert!(t.contains("hl.on(\"hyprland.start\", function() hl.exec_cmd(\"systemctl --user import-environment"));
        assert!(t.contains("|| exec /home/u/.local/bin/nuxut-shell\") hl.exec_cmd(\"systemctl --user is-enabled --quiet nuxut-polkit.service && "));
        assert!(t.contains("&& systemctl --user start nuxut-polkit.service\") end)"));
        assert!(t.contains("hl.exec_cmd(\"systemctl --user stop nuxut-shell.service\") hl.exec_cmd(\"systemctl --user stop nuxut-polkit.service\") end)"));
        let eval = g.eval.unwrap();
        assert!(!eval.contains('\n') && !eval.contains("hl.bind") && eval.starts_with("hl.config({"));
        // Denge: her açılan parantez kapanıyor (kaba sözdizimi denetimi).
        for (o, c) in [('(', ')'), ('{', '}')] {
            assert_eq!(t.matches(o).count(), t.matches(c).count(), "{o}{c}");
        }
    }

    fn input_settings(text: &str) -> InputSettings {
        let (c, w) = crate::config::ShellConfig::parse(text).unwrap();
        assert!(w.is_empty(), "{w:?}");
        c.input.settings
    }

    /// `[input]` Lua'da `hl.config({ input = { … } })` (eval'e de girer); yalnızca yazılı
    /// anahtarlar; dokunmatik alt tablo; anahtar kümesi işaret satırında.
    #[test]
    fn input_table_only_written_keys() {
        let g = generate_lua(&input());
        assert!(!g.text.contains("input = {") && !g.text.contains(INPUT_KEYS_MARKER));
        let s = input_settings(
            "[input.keyboard]\nlayouts = [{ layout = \"tr\" }, { layout = \"us\", variant = \"intl\" }]\noptions = []\nrepeat_rate = 70\n\
             [input.touchpad]\ntap_to_click = false\nscroll_factor = 0.5\nscroll_method = \"edge\"\n",
        );
        let g = generate_lua(&GenerateInput { input: Some(s), ..input() });
        let table = "input = { kb_layout = \"tr,us\", kb_variant = \",intl\", kb_options = \"\", repeat_rate = 70, scroll_method = \"edge\", \
                     touchpad = { tap_to_click = false, scroll_factor = 0.5 } }";
        assert!(g.text.contains(table), "{}", g.text);
        assert!(g.eval.as_deref().unwrap().contains(table));
        assert!(g.text.contains("\n-- nuxut-input: kb_layout kb_variant kb_options repeat_rate touchpad.tap_to_click touchpad.scroll_factor scroll_method\n"));
        // Yalnızca bir anahtar: başka girdi alanı yok.
        let g = generate_lua(&GenerateInput { input: Some(input_settings("[input.mouse]\nsensitivity = -0.25\n")), ..input() });
        assert!(g.text.contains(", input = { sensitivity = -0.25 }"), "{}", g.text);
        for (o, c) in [('(', ')'), ('{', '}')] {
            assert_eq!(g.text.matches(o).count(), g.text.matches(c).count(), "{o}{c}");
        }
    }

    /// Canlı plan: değer değişimi eval, anahtar kalkınca ya da eklenince yeniden okuma.
    #[test]
    fn input_key_set_change_reloads() {
        use super::super::control::ApplyAction;
        use super::super::sync::plan_apply_lua;
        let gen_with = |t: &str| generate_lua(&GenerateInput { input: Some(input_settings(t)), ..input() });
        let a = gen_with("[input.keyboard]\nrepeat_rate = 30\n");
        let b = gen_with("[input.keyboard]\nrepeat_rate = 40\n");
        let none = generate_lua(&input());
        assert!(matches!(plan_apply_lua(Some(&a.text), &b), ApplyAction::Eval(_)));
        assert_eq!(plan_apply_lua(Some(&b.text), &none), ApplyAction::Reload, "varsayılana dön");
        assert_eq!(plan_apply_lua(Some(&none.text), &a), ApplyAction::Reload);
    }

    #[test]
    fn cursor_env_is_in_module_not_eval() {
        let env = vec![("XCURSOR_THEME".to_owned(), "Adwaita".to_owned()), ("HYPRCURSOR_SIZE".to_owned(), "x\")os".to_owned())];
        let g = generate_lua(&GenerateInput { env, ..input() });
        assert!(g.text.contains("\n  hl.env(\"XCURSOR_THEME\", \"Adwaita\")\n"), "{}", g.text);
        assert!(!g.text.contains("HYPRCURSOR_SIZE"));
        assert!(!g.eval.unwrap_or_default().contains("hl.env"));
        // Kabuğun dizini PATH'e yalnızca yoksa (yeniden yüklemede büyümez), systemd/D-Bus'a da.
        assert!(
            g.text.contains(
                "\n  local path = os.getenv(\"PATH\") or \"\"\n  if not (\":\" .. path .. \":\"):find(\":/home/u/.local/bin:\", 1, true) then hl.env(\"PATH\", \"/home/u/.local/bin\" .. \":\" .. path, true) end\n"
            ),
            "{}",
            g.text
        );
    }

    #[test]
    fn groups_off_and_no_launch() {
        let g = generate_lua(&GenerateInput { launch: false, look: None, motion: None, windows: None, hide_defaults: false, ..input() });
        assert!(g.eval.is_none());
        assert!(!g.text.contains("hl.on("));
        assert!(!g.text.contains("hl.config"));
        assert!(!g.text.contains("nuxut.displays"));
        let g = generate_lua(&GenerateInput { displays: Some("~/.config/hypr/nuxut/displays.lua".into()), ..input() });
        assert!(g.text.contains("\npcall(require, \"nuxut.displays\")\n"));
        assert!(!g.eval.unwrap_or_default().contains("displays"), "canlı uygulamaya girmez");

        // Yalnızca zemin: tek başına bir hl.config tablosu (eval'e de gider).
        let g = generate_lua(&GenerateInput { look: None, motion: None, windows: None, ..input() });
        assert_eq!(g.eval.as_deref(), Some("hl.config({ misc = { disable_hyprland_logo = true, force_default_wallpaper = 0 } })"));
    }

    #[test]
    fn comment_stripping_and_requires() {
        let src = "-- require(\"nuxut\")\nlocal s = \"--not comment\"\n--[[ require \"x\" ]] require 'a.b'\npcall(require, \"nuxut\")\n--[==[ multi\nline ]==]\n";
        let (code, comments) = strip_lua_comments(src);
        assert!(code.contains("\"--not comment\""));
        assert_eq!(required_modules(&code), ["a.b", "nuxut"]);
        assert_eq!(required_modules(&comments), ["nuxut", "x"]);
    }

    fn write(p: &Path, s: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap();
    }

    #[test]
    fn chain_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let hypr = tmp.path().join("hypr");
        write(&hypr.join("hyprland.lua"), "require(\"conf.binds\")\nrequire \"looks\"\n-- pcall(require, \"nuxut\")\n");
        write(&hypr.join("conf/binds.lua"), "hl.on(\"hyprland.start\", function() hl.exec_cmd(\"nuxut-shell\") end)\n");
        write(&hypr.join("looks/init.lua"), "pcall(require, 'nuxut')\n");
        let r = scan_lua(&hypr.join("hyprland.lua"), Some(tmp.path()), ScanLimits::default());
        assert_eq!(r.files.len(), 3);
        assert!(r.files[r.included_in.unwrap()].ends_with("looks/init.lua"));
        assert_eq!(r.commented_in, Some(0));
        assert!(r.files[r.external_launch_in.unwrap()].ends_with("conf/binds.lua"));
        assert!(!r.truncated);

        write(&hypr.join("hyprland.lua"), "hl.bind(\"SUPER + V\", hl.dsp.exec_cmd(\"nuxut-shell msg panel toggle right\"))\n");
        let r = scan_lua(&hypr.join("hyprland.lua"), Some(tmp.path()), ScanLimits::default());
        assert_eq!(r.external_launch_in, None);
        assert_eq!(r.included_in, None);
    }
}
