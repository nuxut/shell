//! Başlatıcı hesap makinesi: küçük, saf Rust aritmetik ifade değerlendirici.
//!
//! Neden crate değil: ihtiyaç dört işlem, üs, birkaç fonksiyon; asıl gereken **sınırlar**
//! (girdi uzunluğu, iç içe derinlik, faktöriyel/üs patlaması yok, döngü yok → süre girdi
//! uzunluğuyla doğrusal) ve Türkçe ondalık virgül. `evalexpr` (yeni sürümleri AGPL), `meval`/
//! `fasteval` (bakımsız) ve `exmex` (genel amaçlı, sınır denetimi bizde değil) bu ikisini
//! birlikte vermiyor; libqalculate C++ ve ağır.
//!
//! Dil: sayılar (`3.5`, `3,5`, `1e3`), `+ - * / % ^` (`×`, `÷`, `**` eşdeğer), parantez, tekli
//! `-`/`+`, sonek `!` (tamsayı ≤ 170), sabitler `pi π e tau`, tek argümanlı fonksiyonlar
//! `sqrt cbrt abs sin cos tan asin acos atan ln log log2 exp floor ceil round` (radyan).
//! `^` sağdan birleşir ve tekli eksiden sıkı bağlar (`-2^2 = -4`).

/// Girdi sınırı (bayt).
pub const MAX_INPUT: usize = 128;
/// İç içe parantez/üs/tekli işleç derinliği.
pub const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, Copy)]
enum Token {
    Num(f64),
    Op(char),
    Open,
    Close,
    Func(fn(f64) -> f64),
}

/// İfade gibi görünüyorsa değerlendirir; sonuç sonlu değilse ya da ifade değilse `None`.
/// Tek başına sayı ya da sabit (`42`, `e`) hesap sayılmaz; `=` önekiyle zorlanabilir (`=42`).
pub fn evaluate(input: &str) -> Option<f64> {
    let input = input.trim();
    let (input, forced) = match input.strip_prefix('=') {
        Some(rest) => (rest, true),
        None => (input, false),
    };
    if input.is_empty() || input.len() > MAX_INPUT {
        return None;
    }
    let tokens = tokenize(input)?;
    // İşleç, parantez ya da fonksiyon gerekir: "e", "pi", "42" yazarken hesap satırı çıkmaz.
    let is_expression = tokens.iter().any(|t| !matches!(t, Token::Num(_)));
    if !forced && !is_expression {
        return None;
    }
    let mut parser = Parser { tokens: &tokens, pos: 0, depth: 0 };
    let value = parser.expr()?;
    (parser.pos == tokens.len() && value.is_finite()).then_some(if value == 0.0 { 0.0 } else { value })
}

/// Sonucu gösterim için biçimlendirir: tamsayıysa (|x| < 1e15) tam, değilse en çok 12 anlamlı
/// basamak, sondaki sıfırlar atılır; `decimal` ondalık ayırıcı.
pub fn format(value: f64, decimal: char) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return format!("{}", value as i64);
    }
    let magnitude = value.abs().log10().floor() as i32;
    let text = if !(-6..15).contains(&magnitude) {
        format!("{value:.11e}")
    } else {
        let decimals = (11 - magnitude).clamp(0, 15) as usize;
        format!("{value:.decimals$}")
    };
    let (mantissa, exponent) = match text.split_once('e') {
        Some((m, e)) => (m.to_owned(), Some(e.to_owned())),
        None => (text, None),
    };
    let mantissa = if mantissa.contains('.') { mantissa.trim_end_matches('0').trim_end_matches('.') } else { &mantissa };
    let mut out = mantissa.replace('.', &decimal.to_string());
    if let Some(e) = exponent {
        out.push('e');
        out.push_str(&e);
    }
    out
}

fn tokenize(input: &str) -> Option<Vec<Token>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' => i += 1,
            '0'..='9' | '.' | ',' => {
                let start = i;
                let mut text = String::new();
                let mut seen_sep = false;
                while i < chars.len() {
                    match chars[i] {
                        d @ '0'..='9' => text.push(d),
                        '.' | ',' if !seen_sep => {
                            seen_sep = true;
                            text.push('.');
                        }
                        'e' | 'E' if i > start && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit() || *n == '-' || *n == '+') => {
                            text.push('e');
                            i += 1;
                            text.push(chars[i]);
                            while chars.get(i + 1).is_some_and(char::is_ascii_digit) {
                                i += 1;
                                text.push(chars[i]);
                            }
                        }
                        _ => break,
                    }
                    i += 1;
                }
                tokens.push(Token::Num(text.parse().ok()?));
            }
            '+' | '-' | '/' | '%' | '^' | '!' => {
                tokens.push(Token::Op(c));
                i += 1;
            }
            '*' => {
                if chars.get(i + 1) == Some(&'*') {
                    tokens.push(Token::Op('^'));
                    i += 2;
                } else {
                    tokens.push(Token::Op('*'));
                    i += 1;
                }
            }
            '×' | '·' => {
                tokens.push(Token::Op('*'));
                i += 1;
            }
            '÷' => {
                tokens.push(Token::Op('/'));
                i += 1;
            }
            '(' => {
                tokens.push(Token::Open);
                i += 1;
            }
            ')' => {
                tokens.push(Token::Close);
                i += 1;
            }
            'π' => {
                tokens.push(Token::Num(std::f64::consts::PI));
                i += 1;
            }
            c if c.is_ascii_alphabetic() => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric()) {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect::<String>().to_ascii_lowercase();
                tokens.push(match word.as_str() {
                    "pi" => Token::Num(std::f64::consts::PI),
                    "tau" => Token::Num(std::f64::consts::TAU),
                    "e" => Token::Num(std::f64::consts::E),
                    "sqrt" => Token::Func(f64::sqrt),
                    "cbrt" => Token::Func(f64::cbrt),
                    "abs" => Token::Func(f64::abs),
                    "sin" => Token::Func(f64::sin),
                    "cos" => Token::Func(f64::cos),
                    "tan" => Token::Func(f64::tan),
                    "asin" => Token::Func(f64::asin),
                    "acos" => Token::Func(f64::acos),
                    "atan" => Token::Func(f64::atan),
                    "ln" => Token::Func(f64::ln),
                    "log" => Token::Func(f64::log10),
                    "log2" => Token::Func(f64::log2),
                    "exp" => Token::Func(f64::exp),
                    "floor" => Token::Func(f64::floor),
                    "ceil" => Token::Func(f64::ceil),
                    "round" => Token::Func(f64::round),
                    _ => return None,
                });
            }
            _ => return None,
        }
    }
    Some(tokens)
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<Token> {
        self.tokens.get(self.pos).copied()
    }

    fn enter(&mut self) -> Option<()> {
        self.depth += 1;
        (self.depth <= MAX_DEPTH).then_some(())
    }

    /// expr := term (('+' | '-') term)*
    fn expr(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        while let Some(Token::Op(op @ ('+' | '-'))) = self.peek() {
            self.pos += 1;
            let rhs = self.term()?;
            value = if op == '+' { value + rhs } else { value - rhs };
        }
        Some(value)
    }

    /// term := unary (('*' | '/' | '%') unary)*
    fn term(&mut self) -> Option<f64> {
        let mut value = self.unary()?;
        while let Some(Token::Op(op @ ('*' | '/' | '%'))) = self.peek() {
            self.pos += 1;
            let rhs = self.unary()?;
            value = match op {
                '*' => value * rhs,
                '/' => value / rhs,
                _ => value % rhs,
            };
        }
        Some(value)
    }

    /// unary := ('-' | '+') unary | power
    fn unary(&mut self) -> Option<f64> {
        match self.peek() {
            Some(Token::Op(op @ ('-' | '+'))) => {
                self.pos += 1;
                self.enter()?;
                let v = self.unary()?;
                self.depth -= 1;
                Some(if op == '-' { -v } else { v })
            }
            _ => self.power(),
        }
    }

    /// power := postfix ('^' unary)?   (sağdan birleşir)
    fn power(&mut self) -> Option<f64> {
        let base = self.postfix()?;
        if let Some(Token::Op('^')) = self.peek() {
            self.pos += 1;
            self.enter()?;
            let exp = self.unary()?;
            self.depth -= 1;
            return Some(base.powf(exp));
        }
        Some(base)
    }

    /// postfix := primary '!'*
    fn postfix(&mut self) -> Option<f64> {
        let mut value = self.primary()?;
        while let Some(Token::Op('!')) = self.peek() {
            self.pos += 1;
            value = factorial(value)?;
        }
        Some(value)
    }

    /// primary := number | '(' expr ')' | func primary
    fn primary(&mut self) -> Option<f64> {
        let token = self.peek()?;
        self.pos += 1;
        match token {
            Token::Num(n) => Some(n),
            Token::Open => {
                self.enter()?;
                let v = self.expr()?;
                self.depth -= 1;
                matches!(self.peek()?, Token::Close).then(|| self.pos += 1)?;
                Some(v)
            }
            Token::Func(f) => {
                self.enter()?;
                let arg = self.postfix()?;
                self.depth -= 1;
                Some(f(arg))
            }
            _ => None,
        }
    }
}

fn factorial(n: f64) -> Option<f64> {
    if n.fract() != 0.0 || !(0.0..=170.0).contains(&n) {
        return None;
    }
    Some((1..=n as u32).fold(1.0, |acc, k| acc * k as f64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> Option<f64> {
        evaluate(s)
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(ev("1+2*3"), Some(7.0));
        assert_eq!(ev("(1+2)*3"), Some(9.0));
        assert_eq!(ev("2^3^2"), Some(512.0), "sağdan birleşir");
        assert_eq!(ev("-2^2"), Some(-4.0));
        assert_eq!(ev("2^-1"), Some(0.5));
        assert_eq!(ev("7 % 3"), Some(1.0));
        assert_eq!(ev("3,5 × 2"), Some(7.0), "ondalık virgül");
        assert_eq!(ev("10 ÷ 4"), Some(2.5));
        assert_eq!(ev("2**10"), Some(1024.0));
        assert_eq!(ev("1e3 + 1"), Some(1001.0));
        assert_eq!(ev("5!"), Some(120.0));
        assert_eq!(ev("sqrt 16 + abs(-2)"), Some(6.0));
        assert_eq!(ev("round(pi*100)/100"), Some(314.0 / 100.0));
        assert_eq!(ev("log 1000"), Some(3.0));
        assert_eq!(ev("-0*1"), Some(0.0));
    }

    #[test]
    fn rejects_non_expressions_and_bad_input() {
        assert_eq!(ev("42"), None, "tek sayı hesap değil");
        assert_eq!(ev("=42"), Some(42.0));
        assert_eq!(ev("firefox"), None);
        assert_eq!(ev("e"), None, "yalnız sabit de hesap değil");
        assert_eq!(ev("=pi"), Some(std::f64::consts::PI));
        assert_eq!(ev("1/0"), None, "sonsuz");
        assert_eq!(ev("sqrt(-1)"), None, "NaN");
        assert_eq!(ev("(1+2"), None);
        assert_eq!(ev("1+"), None);
        assert_eq!(ev("1 2"), None);
        assert_eq!(ev("171!"), None);
        assert_eq!(ev("2.5!"), None);
        assert_eq!(ev("1.2.3"), None);
        assert_eq!(ev("10^400"), None);
        assert_eq!(ev(""), None);
        assert_eq!(ev("system(1)"), None);
    }

    #[test]
    fn limits() {
        let deep = format!("{}1{}", "(".repeat(MAX_DEPTH + 1), ")".repeat(MAX_DEPTH + 1));
        assert_eq!(ev(&deep), None);
        let ok = format!("{}1+1{}", "(".repeat(10), ")".repeat(10));
        assert_eq!(ev(&ok), Some(2.0));
        assert_eq!(ev(&"-".repeat(100)), None);
        assert_eq!(ev(&format!("1{}", "+1".repeat(MAX_INPUT))), None, "uzunluk sınırı");
        assert_eq!(ev("3!!!"), None, "720! taşar");
    }

    #[test]
    fn formatting() {
        assert_eq!(format(7.0, ','), "7");
        assert_eq!(format(-1234.0, '.'), "-1234");
        assert_eq!(format(0.1 + 0.2, ','), "0,3");
        assert_eq!(format(2.5, '.'), "2.5");
        assert_eq!(format(1.0 / 3.0, '.'), "0.333333333333");
        assert_eq!(format(1e20, '.'), "1e20");
        assert_eq!(format(1.5e-9, '.'), "1.5e-9");
    }
}
