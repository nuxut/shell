//! PAM conversation fonksiyonu.
//!
//! Kurallar: `PROMPT_ECHO_OFF` → şifre (istek başına **bir kez**; ikinci istem OTP/çok adımlı
//! akıştır, desteklenmez → `PAM_CONV_ERR`), `PROMPT_ECHO_ON` → boş yanıt (kullanıcı adı
//! `pam_start`'ta verildi), `ERROR_MSG`/`TEXT_INFO` → arayüze iletilecek temiz mesaj. Yanıt
//! dizisi `calloc`, yanıt metinleri `malloc` ile ayrılır; sahipliği PAM'e geçer (`free` eder).
//! Hata yolunda o ana kadar ayrılan yanıtlar sıfırlanıp serbest bırakılır. Bu fonksiyon panic
//! etmez (yalnızca bellek tükenmesi; `extern "C"` sınırında panic zaten abort'tur).

use libc::{c_char, c_int, c_void};

use crate::ffi::*;

/// Arayüze gösterilecek mesaj sayısı ve uzunluğu üst sınırı.
pub const MAX_MESSAGES: usize = 4;
pub const MAX_MESSAGE_BYTES: usize = 256;

/// PAM modülünün kullanıcıya mesajı (ör. pam_faillock: "hesap kilitlendi").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PamMessage {
    pub kind: MessageKind,
    /// Kontrol karakterlerinden arındırılmış, ≤ [`MAX_MESSAGE_BYTES`] bayt, boş değil.
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    Error,
    Info,
}

/// `appdata_ptr`'nin gösterdiği durum. `pam_start`'tan `pam_end`'e kadar yaşar.
pub(crate) struct ConvState<'a> {
    secret: &'a [u8],
    answered: bool,
    pub messages: Vec<PamMessage>,
    /// Desteklenmeyen istem görüldü (ikinci şifre istemi, ikili istem…).
    pub unsupported: bool,
}

impl<'a> ConvState<'a> {
    pub fn new(secret: &'a [u8]) -> Self {
        Self {
            secret,
            answered: false,
            messages: Vec::new(),
            unsupported: false,
        }
    }

    fn push_message(&mut self, kind: MessageKind, raw: &[u8]) {
        if self.messages.len() >= MAX_MESSAGES {
            return;
        }
        let text = sanitize(raw);
        if !text.is_empty() {
            self.messages.push(PamMessage { kind, text });
        }
    }
}

/// Mesajı tek satırlık, sınırlı, yazdırılabilir metne çevirir.
pub(crate) fn sanitize(raw: &[u8]) -> String {
    let lossy = String::from_utf8_lossy(raw);
    let mut out = String::new();
    let mut space = false;
    for c in lossy.chars() {
        let c = if c.is_control() || c.is_whitespace() {
            ' '
        } else {
            c
        };
        if c == ' ' {
            if !out.is_empty() {
                space = true;
            }
            continue;
        }
        let extra = c.len_utf8() + usize::from(space);
        if out.len() + extra > MAX_MESSAGE_BYTES {
            break;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(c);
    }
    out
}

/// `pam_conv.conv`.
///
/// # Safety
/// Linux-PAM sözleşmesiyle çağrılmalı: `msg` `num_msg` adet geçerli işaretçi, `resp` yazılabilir,
/// `appdata_ptr` canlı ve başka yerde ödünç alınmamış bir `ConvState`.
pub(crate) unsafe extern "C" fn conversation(
    num_msg: c_int,
    msg: *mut *const pam_message,
    resp: *mut *mut pam_response,
    appdata_ptr: *mut c_void,
) -> c_int {
    if num_msg <= 0
        || num_msg > PAM_MAX_NUM_MSG
        || msg.is_null()
        || resp.is_null()
        || appdata_ptr.is_null()
    {
        return PAM_CONV_ERR;
    }
    // SAFETY: appdata_ptr `authenticate`'in yığınındaki `ConvState`; PAM conversation'ı
    // pam_authenticate sırasında aynı thread'de, eşzamanlı olmadan çağırır ve o sırada durum
    // başka hiçbir yerden erişilmez.
    let state = unsafe { &mut *appdata_ptr.cast::<ConvState<'_>>() };
    let n = num_msg as usize;
    // SAFETY: calloc'un sözleşmesi yok; sonuç null için denetlenir. Sıfır dolgu = tüm `resp` null.
    let replies = unsafe { libc::calloc(n, size_of::<pam_response>()) }.cast::<pam_response>();
    if replies.is_null() {
        return PAM_BUF_ERR;
    }
    for i in 0..n {
        // SAFETY: Linux-PAM `num_msg` uzunluğunda işaretçi dizisi verir; i < n.
        let m = unsafe { *msg.add(i) };
        if m.is_null() {
            // SAFETY: replies bizim calloc'umuz, n eleman, yanıtlar ya null ya bizim malloc'umuz.
            unsafe { free_replies(replies, n) };
            return PAM_CONV_ERR;
        }
        // SAFETY: null değil; PAM mesajı çağrı boyunca geçerli tutar.
        let m = unsafe { &*m };
        let reply = match m.msg_style {
            PAM_PROMPT_ECHO_OFF => {
                if state.answered {
                    state.unsupported = true;
                    // SAFETY: yukarıdakiyle aynı.
                    unsafe { free_replies(replies, n) };
                    return PAM_CONV_ERR;
                }
                state.answered = true;
                c_string_copy(state.secret)
            }
            PAM_PROMPT_ECHO_ON => c_string_copy(b""),
            PAM_ERROR_MSG | PAM_TEXT_INFO => {
                let kind = if m.msg_style == PAM_ERROR_MSG {
                    MessageKind::Error
                } else {
                    MessageKind::Info
                };
                if !m.msg.is_null() {
                    // SAFETY: msg NUL ile biten C dizgisi; strnlen en çok PAM_MAX_MSG_SIZE okur.
                    let len = unsafe { libc::strnlen(m.msg, PAM_MAX_MSG_SIZE) };
                    // SAFETY: [msg, msg+len) okunabilir (strnlen az önce okudu).
                    let raw = unsafe { std::slice::from_raw_parts(m.msg.cast::<u8>(), len) };
                    state.push_message(kind, raw);
                }
                continue;
            }
            _ => {
                state.unsupported = true;
                // SAFETY: yukarıdakiyle aynı.
                unsafe { free_replies(replies, n) };
                return PAM_CONV_ERR;
            }
        };
        if reply.is_null() {
            // SAFETY: yukarıdakiyle aynı.
            unsafe { free_replies(replies, n) };
            return PAM_BUF_ERR;
        }
        // SAFETY: i < n, calloc'lu dizinin içinde.
        unsafe { (*replies.add(i)).resp = reply };
    }
    // SAFETY: resp null değil; dizinin sahipliği PAM'e geçer (free ile serbest bırakır).
    unsafe { *resp = replies };
    PAM_SUCCESS
}

/// `malloc`'lu, NUL ile biten kopya; tahsis başarısızsa null.
fn c_string_copy(bytes: &[u8]) -> *mut c_char {
    // SAFETY: malloc'un sözleşmesi yok; null denetlenir.
    let p = unsafe { libc::malloc(bytes.len() + 1) }.cast::<u8>();
    if p.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: p en az len+1 bayt; kaynak ve hedef örtüşmez.
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        *p.add(bytes.len()) = 0;
    }
    p.cast()
}

/// Yanıtları sıfırlayıp serbest bırakır, sonra diziyi.
///
/// # Safety
/// `replies` `calloc` ile ayrılmış `n` elemanlı dizi; her `resp` null ya da `malloc`'lu C dizgisi.
pub(crate) unsafe fn free_replies(replies: *mut pam_response, n: usize) {
    for i in 0..n {
        // SAFETY: çağıranın sözleşmesi.
        let r = unsafe { (*replies.add(i)).resp };
        if !r.is_null() {
            // SAFETY: r NUL ile biten, yazılabilir malloc'lu dizgi; sıfırlanıp serbest bırakılır.
            unsafe {
                libc::explicit_bzero(r.cast(), libc::strlen(r));
                libc::free(r.cast());
            }
        }
    }
    // SAFETY: calloc'lu dizi.
    unsafe { libc::free(replies.cast()) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    /// Mesajları conversation'a verir; başarılıysa yanıtları (kopya) döndürür ve serbest bırakır.
    fn run(
        state: &mut ConvState<'_>,
        msgs: &[(c_int, Option<&str>)],
    ) -> Result<Vec<Option<Vec<u8>>>, c_int> {
        let texts: Vec<Option<CString>> = msgs
            .iter()
            .map(|(_, t)| t.map(|t| CString::new(t).unwrap()))
            .collect();
        let structs: Vec<pam_message> = msgs
            .iter()
            .zip(&texts)
            .map(|((style, _), t)| pam_message {
                msg_style: *style,
                msg: t.as_ref().map_or(std::ptr::null(), |t| t.as_ptr()),
            })
            .collect();
        let mut ptrs: Vec<*const pam_message> = structs.iter().map(|m| m as *const _).collect();
        let mut resp: *mut pam_response = std::ptr::null_mut();
        // SAFETY: test, PAM sözleşmesini taklit eder: geçerli dizi, canlı durum.
        let rc = unsafe {
            conversation(
                ptrs.len() as c_int,
                ptrs.as_mut_ptr(),
                &mut resp,
                (state as *mut ConvState<'_>).cast(),
            )
        };
        if rc != PAM_SUCCESS {
            assert!(resp.is_null(), "hata yolunda yanıt verilmemeli");
            return Err(rc);
        }
        let out = (0..msgs.len())
            .map(|i| {
                // SAFETY: test; conversation n elemanlı dizi döndürdü.
                let r = unsafe { (*resp.add(i)).resp };
                // SAFETY: test; null değilse NUL ile biten dizgi.
                (!r.is_null()).then(|| unsafe { CStr::from_ptr(r) }.to_bytes().to_vec())
            })
            .collect();
        // SAFETY: test; PAM'in yapacağı serbest bırakma.
        unsafe { free_replies(resp, msgs.len()) };
        Ok(out)
    }

    #[test]
    fn password_prompt_gets_secret_once() {
        let mut st = ConvState::new(b"s3cret");
        let r = run(&mut st, &[(PAM_PROMPT_ECHO_OFF, Some("Password: "))]).unwrap();
        assert_eq!(r, vec![Some(b"s3cret".to_vec())]);
        assert_eq!(
            run(&mut st, &[(PAM_PROMPT_ECHO_OFF, Some("Again: "))]),
            Err(PAM_CONV_ERR)
        );
        assert!(st.unsupported);
    }

    #[test]
    fn second_prompt_in_same_call_fails_and_frees() {
        let mut st = ConvState::new(b"pw");
        let r = run(
            &mut st,
            &[
                (PAM_PROMPT_ECHO_OFF, Some("a")),
                (PAM_PROMPT_ECHO_OFF, Some("b")),
            ],
        );
        assert_eq!(r, Err(PAM_CONV_ERR));
    }

    #[test]
    fn echo_on_gets_empty_and_messages_are_collected() {
        let mut st = ConvState::new(b"pw");
        let r = run(
            &mut st,
            &[
                (PAM_TEXT_INFO, Some("Bilgi")),
                (PAM_PROMPT_ECHO_ON, Some("login: ")),
                (
                    PAM_ERROR_MSG,
                    Some("The account is locked\n due to 3 failed logins."),
                ),
                (PAM_ERROR_MSG, None),
            ],
        )
        .unwrap();
        assert_eq!(r, vec![None, Some(Vec::new()), None, None]);
        assert_eq!(
            st.messages,
            vec![
                PamMessage {
                    kind: MessageKind::Info,
                    text: "Bilgi".into()
                },
                PamMessage {
                    kind: MessageKind::Error,
                    text: "The account is locked due to 3 failed logins.".into()
                },
            ]
        );
    }

    #[test]
    fn unknown_style_and_null_message_fail() {
        let mut st = ConvState::new(b"pw");
        assert_eq!(run(&mut st, &[(7, Some("binary"))]), Err(PAM_CONV_ERR));
        let mut st = ConvState::new(b"pw");
        let mut ptrs: Vec<*const pam_message> = vec![std::ptr::null()];
        let mut resp = std::ptr::null_mut();
        // SAFETY: test; null mesaj işaretçisi reddedilmeli.
        let rc = unsafe {
            conversation(
                1,
                ptrs.as_mut_ptr(),
                &mut resp,
                (&mut st as *mut ConvState<'_>).cast(),
            )
        };
        assert_eq!(rc, PAM_CONV_ERR);
        // SAFETY: test; geçersiz sayılar ve null argümanlar.
        unsafe {
            assert_eq!(
                conversation(
                    0,
                    ptrs.as_mut_ptr(),
                    &mut resp,
                    (&mut st as *mut ConvState<'_>).cast()
                ),
                PAM_CONV_ERR
            );
            assert_eq!(
                conversation(1, ptrs.as_mut_ptr(), &mut resp, std::ptr::null_mut()),
                PAM_CONV_ERR
            );
            assert_eq!(
                conversation(
                    1,
                    std::ptr::null_mut(),
                    &mut resp,
                    (&mut st as *mut ConvState<'_>).cast()
                ),
                PAM_CONV_ERR
            );
        }
    }

    #[test]
    fn message_count_is_bounded() {
        let mut st = ConvState::new(b"");
        let msgs: Vec<(c_int, Option<&str>)> =
            (0..10).map(|_| (PAM_TEXT_INFO, Some("x"))).collect();
        run(&mut st, &msgs).unwrap();
        assert_eq!(st.messages.len(), MAX_MESSAGES);
    }

    #[test]
    fn sanitize_strips_controls_and_bounds_length() {
        assert_eq!(sanitize(b"  a\x1b[31m\tb\r\n "), "a [31m b");
        assert_eq!(sanitize(b"\xff ok"), "\u{fffd} ok");
        let long = "ş".repeat(300);
        let s = sanitize(long.as_bytes());
        assert!(s.len() <= MAX_MESSAGE_BYTES && s.chars().all(|c| c == 'ş'));
        assert_eq!(sanitize(b" \n\t"), "");
    }
}
