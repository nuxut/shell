//! Klavye düzeni: şifre doğru düzende yazılmalı. Düzenler sistem ayarından gelir
//! (`localectl set-x11-keymap`, başlatıcı `NUXUT_GREETER_KB_LAYOUTS`'a doğrulanmış virgüllü liste
//! olarak verir); giriş ekranı etkin düzeni gösterir ve birden çok düzen varsa sıradakine geçer.
//! Geçiş compositor'a (Hyprland `switchxkblayout`) ayrı thread'de gönderilir; Hyprland dışında
//! (test compositor'ı) yalnız etiket değişir.

/// Başlatıcının düzen listesini verdiği ortam değişkeni.
pub const LAYOUTS_ENV: &str = "NUXUT_GREETER_KB_LAYOUTS";
/// En çok bu kadar düzen (XKB grup sınırı 4; fazlası yok sayılır).
pub const MAX_LAYOUTS: usize = 4;
const MAX_CODE: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layouts {
    codes: Vec<String>,
    current: usize,
}

/// Tek düzen kodu: `[a-z0-9_-]`, 1..=16 bayt (ör. `tr`, `us`, `de_nodeadkeys`).
pub fn valid_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= MAX_CODE
        && code.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

impl Layouts {
    /// Virgüllü listeden; geçersiz kodlar atlanır. Boşsa `None` (düğme gösterilmez).
    pub fn parse(list: &str) -> Option<Self> {
        let codes: Vec<String> = list
            .split(',')
            .map(str::trim)
            .filter(|c| valid_code(c))
            .take(MAX_LAYOUTS)
            .map(str::to_owned)
            .collect();
        (!codes.is_empty()).then_some(Self { codes, current: 0 })
    }

    pub fn from_env() -> Option<Self> {
        std::env::var(LAYOUTS_ENV).ok().and_then(|v| Self::parse(&v))
    }

    /// Görünen kısa ad (`TR`, `US`).
    pub fn label(&self) -> String {
        self.codes[self.current].to_ascii_uppercase()
    }

    pub fn many(&self) -> bool {
        self.codes.len() > 1
    }

    pub fn current(&self) -> usize {
        self.current
    }

    /// Sıradaki düzene geçer; yeni dizin.
    pub fn advance(&mut self) -> usize {
        self.current = (self.current + 1) % self.codes.len();
        self.current
    }
}

/// Compositor'a etkin düzeni bildir (bloklar; thread'den çağrılır). Hyprland altında değilse
/// hiçbir şey yapılmaz.
pub fn apply(index: usize) {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_none() {
        tracing::debug!(index, "klavye düzeni: Hyprland yok, yalnız etiket");
        return;
    }
    let status = std::process::Command::new("hyprctl")
        .args(["switchxkblayout", "all", &index.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match status {
        Ok(s) if s.success() => tracing::info!(index, "klavye düzeni değişti"),
        Ok(s) => tracing::warn!(index, %s, "klavye düzeni değiştirilemedi"),
        Err(e) => tracing::warn!(index, "hyprctl çalıştırılamadı: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_validated_list() {
        let mut l = Layouts::parse("tr, us,de_nodeadkeys").unwrap();
        assert_eq!(l.label(), "TR");
        assert!(l.many());
        assert_eq!(l.advance(), 1);
        assert_eq!(l.label(), "US");
        assert_eq!(l.advance(), 2);
        assert_eq!(l.advance(), 0, "döngü");
    }

    #[test]
    fn rejects_injection_and_limits() {
        assert!(Layouts::parse("").is_none());
        assert!(Layouts::parse("\"; os.execute(\"x\") --").is_none());
        let l = Layouts::parse("us,TR,a b,tr").unwrap();
        assert_eq!((l.codes.len(), l.label()), (2, "US".into()), "büyük harf ve boşluk geçersiz");
        assert_eq!(Layouts::parse("a,b,c,d,e,f").unwrap().codes.len(), MAX_LAYOUTS);
        assert!(!Layouts::parse("us").unwrap().many());
        assert!(!valid_code(&"x".repeat(17)));
    }
}
