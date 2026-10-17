//! `Exec` anahtarını argv'ye çevirme (Desktop Entry Specification, "The Exec key").
//!
//! Kabuk **yoktur**: değer spec'in tırnak kurallarıyla argümanlara bölünür, alan kodları
//! açılır ve sonuç doğrudan `execve` argv'si olur. Değer kaçışları (`\s`, `\\` ...)
//! ayrıştırıcıda çözülmüştür; burada yalnızca tırnak katmanı işlenir:
//!
//! - Çift tırnak içinde `\"`, `` \` ``, `\$`, `\\` kaçışları.
//! - Tek tırnak spec'te yok ama gerçek dosyalarda görülüyor; GLib gibi birebir alınır.
//! - Tırnak dışındaki `\x` → `x`.
//!
//! Alan kodları: `%f %u` (tek dosya/URL) ve `%F %U` (liste) başlatıcıda dosya olmadığı için
//! kaldırılır (liste kodu yalnız başına bir argüman olmalıdır); `%i` → `--icon <Icon>` (ikon
//! yoksa hiç), `%c` → çevrilmiş ad, `%k` → dosya yolu, `%%` → `%`; eski `%d %D %n %N %v %m`
//! kaldırılır. Bilinmeyen kod hatadır.

use super::desktop::MAX_VALUE;

/// En çok argüman sayısı.
pub const MAX_ARGS: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExecError {
    #[error("Exec boş")]
    Empty,
    #[error("kapanmamış tırnak")]
    UnclosedQuote,
    #[error("geçersiz alan kodu")]
    BadFieldCode,
    #[error("çok fazla argüman")]
    TooManyArgs,
}

/// Alan kodlarının değerleri.
#[derive(Debug, Clone, Copy, Default)]
pub struct FieldValues<'a> {
    pub icon: Option<&'a str>,
    /// Çevrilmiş ad (`%c`).
    pub name: &'a str,
    /// `.desktop` dosyasının yolu (`%k`).
    pub location: &'a str,
}

/// Ham `Exec` değerini argv'ye çevirir.
pub fn expand(exec: &str, fields: &FieldValues<'_>) -> Result<Vec<String>, ExecError> {
    let mut argv = Vec::new();
    for (arg, quoted) in tokenize(exec)? {
        // Yalnız başına liste/tekil dosya kodu: argüman tamamen düşer; `%i` iki argüman olur.
        if !quoted {
            match arg.as_str() {
                "%f" | "%F" | "%u" | "%U" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => continue,
                "%i" => {
                    if let Some(icon) = fields.icon {
                        argv.push("--icon".to_owned());
                        argv.push(icon.to_owned());
                    }
                    continue;
                }
                _ => {}
            }
        }
        let expanded = expand_inline(&arg, fields)?;
        // Tırnaksız ve yalnızca kaldırılan kodlardan oluşan argüman boş kalırsa düşer.
        if !quoted && expanded.is_empty() {
            continue;
        }
        if argv.len() >= MAX_ARGS {
            return Err(ExecError::TooManyArgs);
        }
        argv.push(expanded);
    }
    if argv.first().is_none_or(|a| a.is_empty()) {
        return Err(ExecError::Empty);
    }
    Ok(argv)
}

/// Bir argümanın içindeki alan kodları (gömülü kullanım; `%F %U` gömülüyse hatadır).
fn expand_inline(arg: &str, fields: &FieldValues<'_>) -> Result<String, ExecError> {
    let mut out = String::with_capacity(arg.len());
    let mut chars = arg.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some('f' | 'u' | 'd' | 'D' | 'n' | 'N' | 'v' | 'm' | 'i') => {}
            Some('c') => out.push_str(fields.name),
            Some('k') => out.push_str(fields.location),
            _ => return Err(ExecError::BadFieldCode),
        }
    }
    Ok(out)
}

/// Argümanlara böler; her argüman için tırnaklı mıydı bilgisi.
fn tokenize(exec: &str) -> Result<Vec<(String, bool)>, ExecError> {
    if exec.len() > MAX_VALUE {
        return Err(ExecError::TooManyArgs);
    }
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_arg = false;
    let mut quoted = false;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' => {
                if in_arg {
                    args.push((std::mem::take(&mut current), quoted));
                    in_arg = false;
                    quoted = false;
                }
            }
            '"' => {
                in_arg = true;
                quoted = true;
                loop {
                    match chars.next() {
                        None => return Err(ExecError::UnclosedQuote),
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(e @ ('"' | '`' | '$' | '\\')) => current.push(e),
                            Some(other) => {
                                current.push('\\');
                                current.push(other);
                            }
                            None => return Err(ExecError::UnclosedQuote),
                        },
                        Some(ch) => current.push(ch),
                    }
                }
            }
            '\'' => {
                in_arg = true;
                quoted = true;
                loop {
                    match chars.next() {
                        None => return Err(ExecError::UnclosedQuote),
                        Some('\'') => break,
                        Some(ch) => current.push(ch),
                    }
                }
            }
            '\\' => {
                in_arg = true;
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            c => {
                in_arg = true;
                current.push(c);
            }
        }
        if args.len() > MAX_ARGS {
            return Err(ExecError::TooManyArgs);
        }
    }
    if in_arg {
        args.push((current, quoted));
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: FieldValues<'static> = FieldValues { icon: Some("firefox"), name: "Fire Fox", location: "/a/b.desktop" };

    fn ex(s: &str) -> Result<Vec<String>, ExecError> {
        expand(s, &F)
    }

    #[test]
    fn field_codes() {
        assert_eq!(ex("firefox %u").unwrap(), ["firefox"]);
        assert_eq!(ex("app %F --x %U %d %D %n %N %v %m").unwrap(), ["app", "--x"]);
        assert_eq!(ex("app %i").unwrap(), ["app", "--icon", "firefox"]);
        assert_eq!(expand("app %i", &FieldValues { icon: None, ..F }).unwrap(), ["app"]);
        assert_eq!(ex("app %c %k 100%%").unwrap(), ["app", "Fire Fox", "/a/b.desktop", "100%"]);
        assert_eq!(ex("app --name=%c").unwrap(), ["app", "--name=Fire Fox"]);
        assert_eq!(ex("app --file=%f").unwrap(), ["app", "--file="]);
        assert_eq!(ex("app %x"), Err(ExecError::BadFieldCode));
        assert_eq!(ex("app --x=%F"), Err(ExecError::BadFieldCode), "liste kodu gömülemez");
        assert_eq!(ex("app 50%"), Err(ExecError::BadFieldCode));
    }

    #[test]
    fn quoting() {
        assert_eq!(ex(r#""/opt/My App/bin" --flag "a \"b\" \$HOME \\ \`x\`""#).unwrap(), [
            "/opt/My App/bin",
            "--flag",
            r#"a "b" $HOME \ `x`"#
        ]);
        assert_eq!(ex(r#"sh -c 'echo $1; rm x' "%c""#).unwrap(), ["sh", "-c", "echo $1; rm x", "Fire Fox"]);
        assert_eq!(ex(r#"app "" end"#).unwrap(), ["app", "", "end"], "boş tırnaklı argüman korunur");
        assert_eq!(ex(r"app a\ b").unwrap(), ["app", "a b"]);
        assert_eq!(ex(r#"app "unterminated"#), Err(ExecError::UnclosedQuote));
        assert_eq!(ex("   "), Err(ExecError::Empty));
        assert_eq!(ex("%u"), Err(ExecError::Empty));
        assert_eq!(ex(r#""" x"#), Err(ExecError::Empty));
    }

    #[test]
    fn no_shell_metacharacter_interpretation() {
        // Kabuk yok: `;`, `|`, `$()` ve `>` düz argümandır.
        assert_eq!(ex("app ; rm -rf ~ | cat $(id) > /tmp/x").unwrap(), [
            "app", ";", "rm", "-rf", "~", "|", "cat", "$(id)", ">", "/tmp/x"
        ]);
    }

    #[test]
    fn arg_limit() {
        let many = format!("app {}", "a ".repeat(MAX_ARGS + 5));
        assert_eq!(ex(&many), Err(ExecError::TooManyArgs));
    }
}
