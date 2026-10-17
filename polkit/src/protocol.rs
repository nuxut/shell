//! polkit ajan yardımcısı (`polkit-agent-helper-1`) satır protokolü — saf ayrıştırıcı ve yazıcı.
//!
//! Kaynak (polkit 127, `polkit-org/polkit` etiketi `127`):
//! - `src/polkitagent/polkitagentsession.c` — `polkit_agent_session_initiate`,
//!   `io_watch_have_data`, `polkit_agent_session_response`, `kill_helper`;
//! - `src/polkitagent/polkitagenthelper-pam.c` — `main`, `send_to_helper`,
//!   `conversation_function`;
//! - `src/polkitagent/polkitagenthelperprivate.c` — `read_cookie`.
//!
//! ## Bağlantı
//! 1. **Soket (önce):** `/run/polkit/agent-helper.socket` varsa ona bağlanılır (systemd soket
//!    etkinleştirme, `Accept=yes`; her bağlantıya `polkit-agent-helper-1 --socket-activated`
//!    root olarak, stdin/stdout = soket). Ajan önce `<kullanıcı adı>\n`, sonra `<çerez>\n` yazar.
//!    Yardımcı istemcinin kimliğini `SO_PEERPIDFD` + `SO_PEERCRED` ile alır ve polkitd'ye
//!    `AuthenticationAgentResponse2` bu özneyle gider; bu yüzden bağlantıyı açan süreç
//!    (diyalog) ajanla aynı uid'dedir.
//! 2. **setuid (geri dönüş):** soket yoksa ya da bağlanılamazsa
//!    `/usr/lib/polkit-1/polkit-agent-helper-1 <kullanıcı adı>` başlatılır, stdin'e `<çerez>\n`
//!    yazılır (CVE-2015-4625'ten beri çerez argv'de değil).
//!
//! ## Yardımcı → ajan (her satır `\n` ile biter)
//! Yardımcı her mesajı `"<ANAHTAR> " + g_strescape(metin)` olarak yazar (metnin sonundaki tek
//! `\n` önce atılır); ajan tüm satırı `g_strcompress` ile çözüp önekle eşler:
//! - `PAM_PROMPT_ECHO_OFF <istem>` — gizli yanıt iste (parola);
//! - `PAM_PROMPT_ECHO_ON <istem>` — görünür yanıt iste;
//! - `PAM_ERROR_MSG <metin>`, `PAM_TEXT_INFO <metin>` — göster;
//! - `SUCCESS` — yardımcı polkitd'ye yanıtı iletti; `FAILURE` — başarısız.
//!   Ardından yardımcı çıkar (EOF).
//!
//! Bilinmeyen satır polkit'te de oturumu başarısız bitirir (`complete_session (FALSE)`).
//!
//! ## Ajan → yardımcı
//! İsteme yanıt: `<yanıt>\n` (`polkit_agent_session_response`). Yardımcı `fgets` ile en çok
//! `PAM_MAX_RESP_SIZE - 1` = 511 bayt okur (satır sonu dahil); daha uzun yanıt bir sonraki
//! isteme taşar. Bu yüzden yanıt ≤ [`MAX_RESPONSE_BYTES`] ve `\n`/NUL içeremez.
//!
//! ## polkit'ten bilerek daha katı olanlar
//! - Satır en çok [`MAX_LINE_BYTES`] (polkit sınırsız okur); aşan satır = başarısızlık.
//! - `SUCCESS`/`FAILURE` tam eşleşir (polkit önek eşler: `SUCCESSxyz` de başarı sayılırdı).
//! - Çözülen metin NUL içeremez ve geçerli UTF-8 olmalı; değilse satır reddedilir (kayıpsız
//!   dönüştürme denenmez — bozuk satır = başarısızlık).
//! - Sondaki yalnız `\` (polkit uyarıp keser) hatadır.

/// Yardımcıdan okunan tek satırın üst sınırı (satır sonu hariç, kaçışlı hâliyle).
pub const MAX_LINE_BYTES: usize = 4096;

/// Yardımcıya yazılabilecek yanıtın üst sınırı (satır sonu hariç): `fgets(buf, 512)` en çok
/// 511 bayt okur, biri `\n` için.
pub const MAX_RESPONSE_BYTES: usize = 510;

/// Yardımcıdan gelen, çözülmüş mesaj.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelperMessage {
    /// Yanıt iste. `echo`: yanıt ekranda görünebilir (`PAM_PROMPT_ECHO_ON`).
    Prompt { text: String, echo: bool },
    ErrorMsg(String),
    TextInfo(String),
    Success,
    Failure,
}

/// Ayrıştırma hatası; her biri oturumu başarısız bitirir.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    TooLong,
    /// Geçersiz kaçış (sonda yalnız `\`).
    BadEscape,
    /// Çözülen metinde NUL.
    Nul,
    InvalidUtf8,
    /// Bilinen bir anahtarla başlamıyor.
    Unknown,
}

/// Bir satırı (satır sonu olmadan) ayrıştırır.
pub fn parse_line(line: &[u8]) -> Result<HelperMessage, ParseError> {
    if line.len() > MAX_LINE_BYTES {
        return Err(ParseError::TooLong);
    }
    let raw = unescape(line)?;
    if raw.contains(&0) {
        return Err(ParseError::Nul);
    }
    let text = String::from_utf8(raw).map_err(|_| ParseError::InvalidUtf8)?;
    let rest = |prefix: &str| text.strip_prefix(prefix).map(str::to_owned);
    if let Some(s) = rest("PAM_PROMPT_ECHO_OFF ") {
        Ok(HelperMessage::Prompt {
            text: s,
            echo: false,
        })
    } else if let Some(s) = rest("PAM_PROMPT_ECHO_ON ") {
        Ok(HelperMessage::Prompt { text: s, echo: true })
    } else if let Some(s) = rest("PAM_ERROR_MSG ") {
        Ok(HelperMessage::ErrorMsg(s))
    } else if let Some(s) = rest("PAM_TEXT_INFO ") {
        Ok(HelperMessage::TextInfo(s))
    } else if text == "SUCCESS" {
        Ok(HelperMessage::Success)
    } else if text == "FAILURE" {
        Ok(HelperMessage::Failure)
    } else {
        Err(ParseError::Unknown)
    }
}

/// GLib `g_strcompress` karşılığı: `\b \f \n \r \t \v \\ \"`, 1–3 haneli sekizlik `\NNN`;
/// bilinmeyen kaçışta ters bölü düşer, karakter kalır (GLib davranışı).
pub fn unescape(s: &[u8]) -> Result<Vec<u8>, ParseError> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        i += 1;
        if c != b'\\' {
            out.push(c);
            continue;
        }
        let Some(&e) = s.get(i) else {
            return Err(ParseError::BadEscape);
        };
        i += 1;
        match e {
            b'0'..=b'7' => {
                let mut v: u32 = u32::from(e - b'0');
                let mut n = 1;
                while n < 3 {
                    match s.get(i) {
                        Some(&d @ b'0'..=b'7') => {
                            v = v * 8 + u32::from(d - b'0');
                            i += 1;
                            n += 1;
                        }
                        _ => break,
                    }
                }
                // GLib `*q++ = v` (gchar'a kırpar); \400 üstü yalnızca bozuk yardımcıdan gelir.
                out.push((v & 0xff) as u8);
            }
            b'b' => out.push(0x08),
            b'f' => out.push(0x0c),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'v' => out.push(0x0b),
            other => out.push(other),
        }
    }
    Ok(out)
}

/// GLib `g_strescape(s, NULL)` karşılığı (sahte yardımcı ve testler için): `\b \f \n \r \t \v
/// \\ \"`, diğer `< 0x20` ve `>= 0x7f` baytlar sekizlik.
pub fn escape(s: &[u8]) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s {
        match b {
            0x08 => out.push_str("\\b"),
            0x0c => out.push_str("\\f"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0x0b => out.push_str("\\v"),
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            b if !(0x20..0x7f).contains(&b) => {
                out.push('\\');
                out.push(char::from(b'0' + (b >> 6)));
                out.push(char::from(b'0' + ((b >> 3) & 7)));
                out.push(char::from(b'0' + (b & 7)));
            }
            b => out.push(char::from(b)),
        }
    }
    out
}

/// Yanıt yazılabilir mi: boyut sınırı, `\n` ve NUL yok. (Boş yanıt geçerli: PAM `nullok`.)
pub fn response_allowed(secret: &[u8]) -> bool {
    secret.len() <= MAX_RESPONSE_BYTES && !secret.contains(&b'\n') && !secret.contains(&0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<HelperMessage, ParseError> {
        parse_line(s.as_bytes())
    }

    #[test]
    fn all_line_kinds() {
        assert_eq!(
            p("PAM_PROMPT_ECHO_OFF Password: "),
            Ok(HelperMessage::Prompt {
                text: "Password: ".into(),
                echo: false
            })
        );
        assert_eq!(
            p("PAM_PROMPT_ECHO_ON login:"),
            Ok(HelperMessage::Prompt {
                text: "login:".into(),
                echo: true
            })
        );
        assert_eq!(
            p("PAM_ERROR_MSG Hesap kilitli"),
            Ok(HelperMessage::ErrorMsg("Hesap kilitli".into()))
        );
        assert_eq!(
            p("PAM_TEXT_INFO Parmağınızı okutun"),
            Ok(HelperMessage::TextInfo("Parmağınızı okutun".into()))
        );
        assert_eq!(p("SUCCESS"), Ok(HelperMessage::Success));
        assert_eq!(p("FAILURE"), Ok(HelperMessage::Failure));
        // Boş istem metni geçerli (anahtar + boşluk).
        assert_eq!(
            p("PAM_PROMPT_ECHO_OFF "),
            Ok(HelperMessage::Prompt {
                text: String::new(),
                echo: false
            })
        );
    }

    #[test]
    fn helper_escaping_roundtrip() {
        // Yardımcı UTF-8 baytlarını sekizlik kaçışla yazar.
        let msg = "Parmağınızı \"okutun\"\tşimdi\\";
        let line = format!("PAM_TEXT_INFO {}", escape(msg.as_bytes()));
        assert!(line.is_ascii());
        assert_eq!(parse_line(line.as_bytes()), Ok(HelperMessage::TextInfo(msg.into())));
        // GLib örnekleri.
        assert_eq!(unescape(b"a\\nb\\101\\7").unwrap(), b"a\nbA\x07");
        assert_eq!(unescape(b"\\q").unwrap(), b"q");
        assert_eq!(unescape(b"\\1234").unwrap(), b"S4");
    }

    #[test]
    fn strict_keywords() {
        for bad in [
            "SUCCESSX",
            "SUCCESS ",
            "success",
            "FAILURE\r",
            "PAM_PROMPT_ECHO_OFF",
            "PAM_PROMPT_ECHO_OFFPassword",
            "",
            " SUCCESS",
            "PAM_FOO bar",
        ] {
            assert_eq!(p(bad), Err(ParseError::Unknown), "{bad:?}");
        }
        // Kaçışla gizlenmiş anahtar da çözülmüş hâliyle eşlenir (polkit gibi).
        assert_eq!(p("\\123UCCESS"), Ok(HelperMessage::Success));
    }

    #[test]
    fn adversarial_lines() {
        let long = format!("PAM_TEXT_INFO {}", "a".repeat(MAX_LINE_BYTES));
        assert_eq!(p(&long), Err(ParseError::TooLong));
        let edge = format!("PAM_TEXT_INFO {}", "a".repeat(MAX_LINE_BYTES - 14));
        assert!(p(&edge).is_ok());
        assert_eq!(p("PAM_TEXT_INFO x\\"), Err(ParseError::BadEscape));
        assert_eq!(p("PAM_TEXT_INFO a\\0b"), Err(ParseError::Nul));
        assert_eq!(p("PAM_TEXT_INFO a\\000"), Err(ParseError::Nul));
        assert_eq!(
            parse_line(b"PAM_TEXT_INFO a\0b"),
            Err(ParseError::Nul)
        );
        // Bozuk UTF-8: ham bayt da kaçışlı bayt da reddedilir.
        assert_eq!(
            parse_line(b"PAM_TEXT_INFO \xff\xfe"),
            Err(ParseError::InvalidUtf8)
        );
        assert_eq!(p("PAM_TEXT_INFO \\377"), Err(ParseError::InvalidUtf8));
        assert_eq!(p("PAM_TEXT_INFO \\303"), Err(ParseError::InvalidUtf8));
        // Gömülü satır sonu metinde kalır (gösterimde temizlenir), protokolü bölmez.
        assert_eq!(
            p("PAM_ERROR_MSG a\\nSUCCESS"),
            Ok(HelperMessage::ErrorMsg("a\nSUCCESS".into()))
        );
    }

    #[test]
    fn response_limits() {
        assert!(response_allowed(b""));
        assert!(response_allowed(&[b'x'; MAX_RESPONSE_BYTES]));
        assert!(!response_allowed(&[b'x'; MAX_RESPONSE_BYTES + 1]));
        assert!(!response_allowed(b"a\nb"));
        assert!(!response_allowed(b"a\0b"));
    }
}
